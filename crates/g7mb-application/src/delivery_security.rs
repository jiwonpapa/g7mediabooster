//! Delivery policy bounds and provider redirect validation.

use std::time::Duration;

use url::Url;

const MIN_DELIVERY_TTL: Duration = Duration::from_secs(30);
const MAX_DELIVERY_TTL: Duration = Duration::from_secs(15 * 60);
const MIN_MANIFEST_CACHE_TTL: Duration = Duration::from_secs(1);
const MAX_MANIFEST_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const MIN_MANIFEST_CACHE_BYTES: u64 = 64 * 1024;
const MAX_MANIFEST_CACHE_BYTES: u64 = 64 * 1024 * 1024;

/// Bounded private-delivery and immutable-manifest cache policy.
#[derive(Clone, Debug)]
pub struct DerivativeDeliveryPolicy {
    /// Provider signature lifetime.
    pub signed_url_ttl: Duration,
    /// Maximum age of an immutable derivative manifest in memory.
    pub manifest_cache_ttl: Duration,
    /// Approximate total bytes admitted to the manifest cache.
    pub manifest_cache_max_bytes: u64,
    /// Exact lower-case provider authorities permitted in signed redirects.
    pub redirect_allowed_authorities: Vec<String>,
}

impl Default for DerivativeDeliveryPolicy {
    fn default() -> Self {
        Self {
            signed_url_ttl: Duration::from_secs(5 * 60),
            manifest_cache_ttl: Duration::from_secs(60),
            manifest_cache_max_bytes: 4 * 1024 * 1024,
            redirect_allowed_authorities: vec!["private.invalid".to_owned()],
        }
    }
}

impl DerivativeDeliveryPolicy {
    pub(crate) fn is_valid(&self) -> bool {
        (MIN_DELIVERY_TTL..=MAX_DELIVERY_TTL).contains(&self.signed_url_ttl)
            && (MIN_MANIFEST_CACHE_TTL..=MAX_MANIFEST_CACHE_TTL).contains(&self.manifest_cache_ttl)
            && self.manifest_cache_ttl <= self.signed_url_ttl
            && (MIN_MANIFEST_CACHE_BYTES..=MAX_MANIFEST_CACHE_BYTES)
                .contains(&self.manifest_cache_max_bytes)
            && !self.redirect_allowed_authorities.is_empty()
            && self.redirect_allowed_authorities.len() <= 8
            && self.redirect_allowed_authorities.iter().all(|authority| {
                !authority.is_empty()
                    && authority.len() <= 512
                    && authority == &authority.to_ascii_lowercase()
                    && authority.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'.' | b'-' | b':' | b'[' | b']')
                    })
            })
    }
}

pub(crate) fn valid_preset(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub(crate) fn valid_tenant(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub(crate) fn trusted_redirect(value: &str, allowed_authorities: &[String]) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
    {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let authority = match url.port() {
        Some(port) if host.contains(':') => format!("[{}]:{port}", host.to_ascii_lowercase()),
        Some(port) => format!("{}:{port}", host.to_ascii_lowercase()),
        None => host.to_ascii_lowercase(),
    };
    allowed_authorities
        .iter()
        .any(|allowed| allowed == &authority)
}

#[cfg(test)]
mod tests {
    use super::{DerivativeDeliveryPolicy, trusted_redirect};

    #[test]
    fn redirect_authority_is_exact_and_plain_http_is_loopback_only() {
        let allowed = vec!["private.invalid".to_owned(), "127.0.0.1:9000".to_owned()];
        assert!(trusted_redirect(
            "https://private.invalid/media/image.jpg?signed=value",
            &allowed
        ));
        assert!(trusted_redirect(
            "http://127.0.0.1:9000/media/image.jpg?signed=value",
            &allowed
        ));
        assert!(!trusted_redirect(
            "https://attacker.invalid/media/image.jpg",
            &allowed
        ));
        assert!(!trusted_redirect(
            "http://private.invalid/media/image.jpg",
            &allowed
        ));
        assert!(DerivativeDeliveryPolicy::default().is_valid());
    }
}
