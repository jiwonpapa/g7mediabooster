//! Bounded public/private delivery settings and provider redirect authorities.

use std::{net::SocketAddr, path::PathBuf};

use config::ConfigError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use url::Url;

use crate::{StorageProvider, StorageSettings, parse_storage_endpoint, resolve_secret};

/// Bounded private and public derivative delivery settings.
#[derive(Clone, Debug, Deserialize)]
pub struct DeliverySettings {
    /// Short-lived provider GET signature lifetime.
    pub signed_url_ttl_seconds: u64,
    /// Maximum immutable manifest age in process memory.
    pub manifest_cache_ttl_seconds: u64,
    /// Approximate total manifest cache weight in bytes.
    pub manifest_cache_max_bytes: u64,
    /// Enables the separate loopback public-thumbnail listener.
    pub public_enabled: bool,
    /// Loopback listener exposed only through an explicit reverse-proxy path.
    pub public_bind_addr: SocketAddr,
    /// HMAC secret for immutable public media URLs.
    #[serde(default = "crate::empty_secret")]
    pub public_signing_secret: SecretString,
    /// Absolute root-owned file containing the public URL signing secret.
    #[serde(default)]
    pub public_signing_secret_file: Option<PathBuf>,
    /// Maximum public URL lifetime accepted by the listener.
    pub public_token_max_ttl_seconds: u64,
    /// Sustained public-thumbnail requests admitted per second.
    pub public_rate_limit_requests_per_second: u32,
    /// Public-thumbnail token-bucket burst.
    pub public_rate_limit_burst: u32,
    /// Maximum public-thumbnail handlers in flight.
    pub public_max_in_flight_requests: usize,
}

pub(crate) fn validate_delivery_settings(
    settings: &DeliverySettings,
    control_bind_addr: SocketAddr,
) -> Result<(), ConfigError> {
    let secret_len = settings.public_signing_secret.expose_secret().len();
    let invalid_secret = if settings.public_enabled {
        !(32..=256).contains(&secret_len)
    } else {
        secret_len != 0 && !(32..=256).contains(&secret_len)
    };
    if !(30..=15 * 60).contains(&settings.signed_url_ttl_seconds)
        || !(1..=5 * 60).contains(&settings.manifest_cache_ttl_seconds)
        || settings.manifest_cache_ttl_seconds > settings.signed_url_ttl_seconds
        || !(64 * 1024..=64 * 1024 * 1024).contains(&settings.manifest_cache_max_bytes)
        || !settings.public_bind_addr.ip().is_loopback()
        || (settings.public_enabled && settings.public_bind_addr == control_bind_addr)
        || !(30..=60 * 60).contains(&settings.public_token_max_ttl_seconds)
        || !(1..=10_000).contains(&settings.public_rate_limit_requests_per_second)
        || settings.public_rate_limit_burst < settings.public_rate_limit_requests_per_second
        || settings.public_rate_limit_burst > 100_000
        || !(1..=1024).contains(&settings.public_max_in_flight_requests)
        || invalid_secret
    {
        return Err(ConfigError::Message(
            "delivery settings violate listener, signature, rate, manifest TTL, or cache limits"
                .to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn resolve_public_secret(settings: &mut DeliverySettings) -> Result<(), ConfigError> {
    if settings.public_enabled {
        resolve_secret(
            &mut settings.public_signing_secret,
            settings.public_signing_secret_file.as_deref(),
            "delivery.public_signing_secret",
        )?;
    }
    Ok(())
}

pub(crate) fn derive_redirect_authorities(
    storage: &StorageSettings,
) -> Result<Vec<String>, ConfigError> {
    let authorities = match storage.provider {
        StorageProvider::R2 | StorageProvider::Generic => {
            let endpoint = storage
                .endpoint_url
                .as_deref()
                .ok_or_else(|| {
                    ConfigError::Message(
                        "custom storage provider has no redirect authority".to_owned(),
                    )
                })
                .and_then(parse_storage_endpoint)?;
            let endpoint_authority = canonical_authority(&endpoint)?;
            if storage.force_path_style {
                vec![endpoint_authority]
            } else {
                let host = endpoint.host_str().ok_or_else(|| {
                    ConfigError::Message("storage endpoint URL has no host".to_owned())
                })?;
                let virtual_host = format!("{}.{}", storage.derivative_bucket, host);
                let virtual_authority = match endpoint.port() {
                    Some(port) => format!("{virtual_host}:{port}"),
                    None => virtual_host,
                };
                vec![endpoint_authority, virtual_authority]
            }
        }
        StorageProvider::AwsS3 | StorageProvider::Lightsail => vec![
            format!(
                "{}.s3.{}.amazonaws.com",
                storage.derivative_bucket, storage.region
            ),
            format!("s3.{}.amazonaws.com", storage.region),
        ],
    };
    if authorities.is_empty()
        || authorities.len() > 4
        || authorities.iter().any(|authority| {
            authority.is_empty()
                || authority.len() > 512
                || !authority.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'-' | b':' | b'[' | b']')
                })
        })
    {
        return Err(ConfigError::Message(
            "derived storage redirect authority is invalid".to_owned(),
        ));
    }
    Ok(authorities)
}

fn canonical_authority(url: &Url) -> Result<String, ConfigError> {
    let host = url
        .host_str()
        .ok_or_else(|| ConfigError::Message("storage endpoint URL has no host".to_owned()))?
        .to_ascii_lowercase();
    Ok(match url.port() {
        Some(port) if host.contains(':') => format!("[{host}]:{port}"),
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::{DeliverySettings, derive_redirect_authorities, validate_delivery_settings};
    use crate::{StorageProvider, StorageSettings};

    #[test]
    fn provider_redirect_authorities_are_exact_and_bucket_scoped()
    -> Result<(), Box<dyn std::error::Error>> {
        let settings = StorageSettings {
            provider: StorageProvider::R2,
            endpoint_url: Some(
                "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com".to_owned(),
            ),
            region: "auto".to_owned(),
            raw_bucket: "g7mb-raw".to_owned(),
            derivative_bucket: "g7mb-media".to_owned(),
            access_key_id: SecretString::from("access".to_owned()),
            access_key_id_file: None,
            secret_access_key: SecretString::from("secret".to_owned()),
            secret_access_key_file: None,
            force_path_style: false,
        };
        assert_eq!(
            derive_redirect_authorities(&settings)?,
            vec![
                "0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com",
                "g7mb-media.0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com",
            ]
        );
        let aws = StorageSettings {
            provider: StorageProvider::AwsS3,
            endpoint_url: None,
            region: "ap-northeast-2".to_owned(),
            ..settings
        };
        assert_eq!(
            derive_redirect_authorities(&aws)?,
            vec![
                "g7mb-media.s3.ap-northeast-2.amazonaws.com",
                "s3.ap-northeast-2.amazonaws.com",
            ]
        );
        Ok(())
    }

    #[test]
    fn public_delivery_requires_loopback_distinct_listener_and_strong_secret()
    -> Result<(), Box<dyn std::error::Error>> {
        let settings = DeliverySettings {
            signed_url_ttl_seconds: 300,
            manifest_cache_ttl_seconds: 60,
            manifest_cache_max_bytes: 4 * 1024 * 1024,
            public_enabled: true,
            public_bind_addr: "127.0.0.1:8088".parse()?,
            public_signing_secret: SecretString::from("short".to_owned()),
            public_signing_secret_file: None,
            public_token_max_ttl_seconds: 300,
            public_rate_limit_requests_per_second: 100,
            public_rate_limit_burst: 200,
            public_max_in_flight_requests: 128,
        };
        assert!(validate_delivery_settings(&settings, "127.0.0.1:8088".parse()?).is_err());
        Ok(())
    }
}
