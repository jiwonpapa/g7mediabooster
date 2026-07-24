//! Separate signed public-thumbnail listener with no control-plane routes.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::bail;
use axum::{
    Router,
    extract::{DefaultBodyLimit, OriginalUri, Path, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse as _, Response},
    routing::get,
};
use g7mb_application::delivery::{DerivativeDeliveryError, DerivativeDeliveryService};
use g7mb_auth::{SignedMediaUrl, verify_media_url};
use g7mb_domain::UploadId;
use metrics::{counter, gauge, histogram};
use secrecy::{ExposeSecret as _, SecretString};
use time::OffsetDateTime;
use tokio::sync::{Mutex, Semaphore};
use tower_http::{
    catch_panic::CatchPanicLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    set_header::SetResponseHeaderLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

/// Bounded admission and token policy for the public-thumbnail listener.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicDeliveryPolicy {
    /// Maximum accepted signature lifetime.
    pub token_max_ttl: Duration,
    /// Sustained requests replenished per second.
    pub requests_per_second: u32,
    /// Short token-bucket burst capacity.
    pub burst: u32,
    /// Maximum public handlers simultaneously in flight.
    pub max_in_flight: usize,
}

impl PublicDeliveryPolicy {
    fn is_valid(self) -> bool {
        (Duration::from_secs(30)..=Duration::from_secs(60 * 60)).contains(&self.token_max_ttl)
            && (1..=10_000).contains(&self.requests_per_second)
            && self.burst >= self.requests_per_second
            && self.burst <= 100_000
            && (1..=1024).contains(&self.max_in_flight)
    }
}

/// Dependencies installed only on the separate public-thumbnail listener.
#[derive(Clone)]
pub struct PublicDeliveryState {
    tenant_id: String,
    delivery: DerivativeDeliveryService,
    signing_secret: SecretString,
    policy: PublicDeliveryPolicy,
    limiter: PublicDeliveryLimiter,
}

impl PublicDeliveryState {
    /// Validates the dedicated public listener state before accepting traffic.
    pub fn new(
        tenant_id: String,
        delivery: DerivativeDeliveryService,
        signing_secret: SecretString,
        policy: PublicDeliveryPolicy,
    ) -> anyhow::Result<Self> {
        if !valid_identifier(&tenant_id, 64)
            || !(32..=256).contains(&signing_secret.expose_secret().len())
            || !policy.is_valid()
        {
            bail!("public derivative delivery policy is invalid");
        }
        Ok(Self {
            tenant_id,
            delivery,
            signing_secret,
            policy,
            limiter: PublicDeliveryLimiter::new(policy),
        })
    }
}

#[derive(Clone)]
struct PublicDeliveryLimiter {
    policy: PublicDeliveryPolicy,
    bucket: Arc<Mutex<PublicTokenBucket>>,
    in_flight: Arc<Semaphore>,
}

impl PublicDeliveryLimiter {
    fn new(policy: PublicDeliveryPolicy) -> Self {
        Self {
            policy,
            bucket: Arc::new(Mutex::new(PublicTokenBucket {
                tokens: u64::from(policy.burst),
                refilled_at: Instant::now(),
            })),
            in_flight: Arc::new(Semaphore::new(policy.max_in_flight)),
        }
    }

    async fn admit(&self) -> bool {
        let mut bucket = self.bucket.lock().await;
        let now = Instant::now();
        let refill = bucket
            .refilled_at
            .elapsed()
            .as_nanos()
            .saturating_mul(u128::from(self.policy.requests_per_second))
            / 1_000_000_000_u128;
        if refill > 0 {
            bucket.tokens = bucket
                .tokens
                .saturating_add(u64::try_from(refill).unwrap_or(u64::MAX))
                .min(u64::from(self.policy.burst));
            bucket.refilled_at = now;
        }
        if bucket.tokens == 0 {
            return false;
        }
        bucket.tokens -= 1;
        true
    }
}

struct PublicTokenBucket {
    tokens: u64,
    refilled_at: Instant,
}

struct PublicInFlightMetric;

impl PublicInFlightMetric {
    fn new() -> Self {
        gauge!("g7mb_public_delivery_in_flight_requests").increment(1.0);
        Self
    }
}

impl Drop for PublicInFlightMetric {
    fn drop(&mut self) {
        gauge!("g7mb_public_delivery_in_flight_requests").decrement(1.0);
    }
}

/// Builds a route-minimal listener that cannot expose control or metrics handlers.
pub fn public_delivery_router(state: PublicDeliveryState) -> Router {
    let request_id = axum::http::HeaderName::from_static("x-request-id");
    let limiter = state.limiter.clone();
    Router::new()
        .route(
            "/media/v1/{tenant_id}/{upload_id}/{preset_id}/{file}",
            get(deliver_public_thumbnail),
        )
        .fallback(public_not_found)
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            limiter,
            enforce_public_limits,
        ))
        .layer(DefaultBodyLimit::max(0))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-store"),
        ))
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(10),
        ))
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
}

async fn enforce_public_limits(
    State(limiter): State<PublicDeliveryLimiter>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() != Method::GET {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !limiter.admit().await {
        counter!("g7mb_public_delivery_admission_rejections_total", "reason" => "rate")
            .increment(1);
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let Ok(permit) = limiter.in_flight.clone().try_acquire_owned() else {
        counter!("g7mb_public_delivery_admission_rejections_total", "reason" => "concurrency")
            .increment(1);
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let metric = PublicInFlightMetric::new();
    let response = next.run(request).await;
    drop(metric);
    drop(permit);
    response
}

async fn deliver_public_thumbnail(
    State(state): State<PublicDeliveryState>,
    Path((tenant_id, upload_id, preset_id, file)): Path<(String, String, String, String)>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let started = Instant::now();
    let result = async {
        if tenant_id != state.tenant_id
            || !valid_identifier(&tenant_id, 64)
            || !valid_identifier(&preset_id, 128)
            || file != "thumbnail.jpg"
        {
            return Err(PublicDeliveryFailure::NotFound);
        }
        let upload_id = upload_id
            .parse::<UploadId>()
            .map_err(|_| PublicDeliveryFailure::NotFound)?;
        let canonical_path = format!("/media/v1/{tenant_id}/{upload_id}/{preset_id}/thumbnail.jpg");
        if uri.path() != canonical_path {
            return Err(PublicDeliveryFailure::NotFound);
        }
        let token =
            parse_token_query(uri.query()).ok_or(PublicDeliveryFailure::InvalidSignature)?;
        let media = SignedMediaUrl {
            path: &canonical_path,
            expires_at: token.expires_at,
            signature: token.signature,
        };
        let now = OffsetDateTime::now_utc().unix_timestamp();
        verify_media_url(
            &media,
            &state.signing_secret,
            now,
            state.policy.token_max_ttl.as_secs(),
        )
        .map_err(|_| PublicDeliveryFailure::InvalidSignature)?;
        let delivery = state
            .delivery
            .presign_exact(&tenant_id, upload_id, &preset_id, "thumbnail")
            .await
            .map_err(PublicDeliveryFailure::from)?;
        if delivery.content_type != "image/jpeg" {
            return Err(PublicDeliveryFailure::NotFound);
        }
        let location = HeaderValue::from_str(delivery.url.expose_secret())
            .map_err(|_| PublicDeliveryFailure::Unavailable)?;
        Ok::<_, PublicDeliveryFailure>(location)
    }
    .await;
    histogram!("g7mb_public_delivery_request_duration_seconds")
        .record(started.elapsed().as_secs_f64());
    match result {
        Ok(location) => {
            counter!("g7mb_public_delivery_requests_total", "result" => "redirect").increment(1);
            (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
        }
        Err(PublicDeliveryFailure::InvalidSignature) => {
            counter!("g7mb_public_delivery_requests_total", "result" => "invalid_signature")
                .increment(1);
            StatusCode::NOT_FOUND.into_response()
        }
        Err(PublicDeliveryFailure::NotFound) => {
            counter!("g7mb_public_delivery_requests_total", "result" => "not_found").increment(1);
            StatusCode::NOT_FOUND.into_response()
        }
        Err(PublicDeliveryFailure::Unavailable) => {
            counter!("g7mb_public_delivery_requests_total", "result" => "unavailable").increment(1);
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

async fn public_not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

struct PublicToken<'a> {
    expires_at: i64,
    signature: &'a str,
}

fn parse_token_query(query: Option<&str>) -> Option<PublicToken<'_>> {
    let mut expires_at = None;
    let mut signature = None;
    for pair in query?.split('&') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "expires" if expires_at.is_none() => {
                expires_at = value.parse::<i64>().ok();
            }
            "signature"
                if signature.is_none()
                    && (43..=128).contains(&value.len())
                    && value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
                    }) =>
            {
                signature = Some(value);
            }
            _ => return None,
        }
    }
    Some(PublicToken {
        expires_at: expires_at?,
        signature: signature?,
    })
}

fn valid_identifier(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

enum PublicDeliveryFailure {
    InvalidSignature,
    NotFound,
    Unavailable,
}

impl From<DerivativeDeliveryError> for PublicDeliveryFailure {
    fn from(error: DerivativeDeliveryError) -> Self {
        match error {
            DerivativeDeliveryError::NotFound
            | DerivativeDeliveryError::NotReady
            | DerivativeDeliveryError::InvalidTenant
            | DerivativeDeliveryError::InvalidVariant => Self::NotFound,
            DerivativeDeliveryError::InvalidPolicy
            | DerivativeDeliveryError::Repository(_)
            | DerivativeDeliveryError::ObjectStore(_)
            | DerivativeDeliveryError::UntrustedRedirect => Self::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use secrecy::{ExposeSecret as _, SecretString};

    use super::{PublicDeliveryPolicy, PublicToken, parse_token_query};

    #[test]
    fn token_query_is_exact_and_rejects_ambiguity() {
        let parsed = parse_token_query(Some(
            "expires=1700000300&signature=abcdefghijklmnopqrstuvwxyzABCDEFGH012345678",
        ));
        assert!(matches!(
            parsed,
            Some(PublicToken {
                expires_at: 1_700_000_300,
                ..
            })
        ));
        assert!(parse_token_query(Some(
            "expires=1700000300&expires=1700000400&signature=abcdefghijklmnopqrstuvwxyzABCDEFGH012345678"
        ))
        .is_none());
        assert!(parse_token_query(Some(
            "expires=1700000300&signature=abcdefghijklmnopqrstuvwxyzABCDEFGH012345678&width=99999"
        ))
        .is_none());
    }

    #[test]
    fn public_policy_requires_a_separate_strong_secret() {
        let policy = PublicDeliveryPolicy {
            token_max_ttl: std::time::Duration::from_secs(300),
            requests_per_second: 100,
            burst: 200,
            max_in_flight: 128,
        };
        assert!(policy.is_valid());
        assert_eq!(
            SecretString::from("separate-32-byte-public-url-secret".to_owned())
                .expose_secret()
                .len(),
            34
        );
    }
}
