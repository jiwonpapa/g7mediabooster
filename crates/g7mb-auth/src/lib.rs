//! HMAC request canonicalization and constant-time verification for PHP adapters.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit as _, Mac};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

/// Canonical immutable media URL covered by a short-lived public signature.
#[derive(Clone, Copy, Debug)]
pub struct SignedMediaUrl<'a> {
    /// Exact percent-encoded path beginning with `/media/v1/`.
    pub path: &'a str,
    /// Absolute Unix expiration included in the query string.
    pub expires_at: i64,
    /// Base64url-no-padding HMAC signature.
    pub signature: &'a str,
}

impl SignedMediaUrl<'_> {
    /// Builds the exact payload shared by PHP URL producers and the Rust delivery listener.
    #[must_use]
    pub fn canonical_payload(&self) -> String {
        format!(
            "G7MB-MEDIA-HMAC-SHA256\nGET\n{}\n{}",
            self.path, self.expires_at
        )
    }
}

/// Signed immutable media URL verification failure.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum MediaUrlVerificationError {
    /// The URL path or expiration is outside the canonical contract.
    #[error("media URL contains an invalid field")]
    InvalidField,
    /// The URL has expired.
    #[error("media URL has expired")]
    Expired,
    /// The requested lifetime exceeds the configured ceiling.
    #[error("media URL lifetime exceeds the configured ceiling")]
    ExcessiveLifetime,
    /// Signature is not valid base64url.
    #[error("media URL signature encoding is invalid")]
    InvalidEncoding,
    /// Signature does not match.
    #[error("media URL signature is invalid")]
    InvalidSignature,
    /// Secret cannot initialize HMAC.
    #[error("media URL verification key is invalid")]
    InvalidKey,
}

/// Inputs covered by the PHP-to-Rust request signature.
#[derive(Clone, Copy, Debug)]
pub struct SignedRequest<'a> {
    /// Public application key identifier.
    pub key_id: &'a str,
    /// Unix timestamp in seconds.
    pub timestamp: i64,
    /// Single-use random nonce.
    pub nonce: &'a str,
    /// Uppercase HTTP method.
    pub method: &'a str,
    /// Path and canonical query string.
    pub path_and_query: &'a str,
    /// Lowercase SHA-256 hex digest of the request body.
    pub body_sha256: &'a str,
    /// Base64url-no-padding HMAC signature.
    pub signature: &'a str,
}

impl SignedRequest<'_> {
    /// Builds the exact canonical payload shared with PHP adapters.
    #[must_use]
    pub fn canonical_payload(&self) -> String {
        format!(
            "G7MB-HMAC-SHA256\n{}\n{}\n{}\n{}\n{}\n{}",
            self.key_id,
            self.timestamp,
            self.nonce,
            self.method,
            self.path_and_query,
            self.body_sha256
        )
    }
}

/// HMAC verification failure safe to map to HTTP 401.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum VerificationError {
    /// Timestamp falls outside the configured skew.
    #[error("request timestamp is outside the allowed clock skew")]
    StaleTimestamp,
    /// Canonical input is malformed.
    #[error("signed request contains an invalid field")]
    InvalidField,
    /// Signature is not valid base64url.
    #[error("request signature encoding is invalid")]
    InvalidEncoding,
    /// Signature does not match.
    #[error("request signature is invalid")]
    InvalidSignature,
    /// Secret cannot initialize HMAC.
    #[error("verification key is invalid")]
    InvalidKey,
}

/// Verifies canonical form, timestamp, and HMAC in constant time.
///
/// Replay prevention is a separate durable step: callers must atomically reserve
/// `(key_id, nonce)` only after this function succeeds.
pub fn verify(
    request: &SignedRequest<'_>,
    secret: &SecretString,
    now_unix_seconds: i64,
    allowed_skew_seconds: i64,
) -> Result<(), VerificationError> {
    if allowed_skew_seconds < 0
        || now_unix_seconds.abs_diff(request.timestamp) > allowed_skew_seconds as u64
    {
        return Err(VerificationError::StaleTimestamp);
    }
    if !valid_field(request) {
        return Err(VerificationError::InvalidField);
    }
    if !(32..=256).contains(&secret.expose_secret().len()) {
        return Err(VerificationError::InvalidKey);
    }

    let signature = URL_SAFE_NO_PAD
        .decode(request.signature)
        .map_err(|_| VerificationError::InvalidEncoding)?;
    let mut mac = HmacSha256::new_from_slice(secret.expose_secret().as_bytes())
        .map_err(|_| VerificationError::InvalidKey)?;
    mac.update(request.canonical_payload().as_bytes());
    mac.verify_slice(&signature)
        .map_err(|_| VerificationError::InvalidSignature)
}

/// Produces the PHP-compatible base64url signature for contract tests and adapters.
pub fn sign(
    request: &SignedRequest<'_>,
    secret: &SecretString,
) -> Result<String, VerificationError> {
    if !valid_field(request) {
        return Err(VerificationError::InvalidField);
    }
    if !(32..=256).contains(&secret.expose_secret().len()) {
        return Err(VerificationError::InvalidKey);
    }
    let mut mac = HmacSha256::new_from_slice(secret.expose_secret().as_bytes())
        .map_err(|_| VerificationError::InvalidKey)?;
    mac.update(request.canonical_payload().as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

/// Computes the lowercase body digest used by the signing contract.
#[must_use]
pub fn sha256_hex(body: &[u8]) -> String {
    hex::encode(Sha256::digest(body))
}

/// Produces the PHP-compatible signature for one immutable public media path.
pub fn sign_media_url(
    media: &SignedMediaUrl<'_>,
    secret: &SecretString,
) -> Result<String, MediaUrlVerificationError> {
    validate_media_url_fields(media)?;
    let mut mac = media_url_mac(secret)?;
    mac.update(media.canonical_payload().as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

/// Verifies an immutable public media path, its bounded expiration, and its HMAC.
pub fn verify_media_url(
    media: &SignedMediaUrl<'_>,
    secret: &SecretString,
    now_unix_seconds: i64,
    max_lifetime_seconds: u64,
) -> Result<(), MediaUrlVerificationError> {
    validate_media_url_fields(media)?;
    if media.expires_at < now_unix_seconds {
        return Err(MediaUrlVerificationError::Expired);
    }
    if media.expires_at.abs_diff(now_unix_seconds) > max_lifetime_seconds {
        return Err(MediaUrlVerificationError::ExcessiveLifetime);
    }
    let signature = URL_SAFE_NO_PAD
        .decode(media.signature)
        .map_err(|_| MediaUrlVerificationError::InvalidEncoding)?;
    let mut mac = media_url_mac(secret)?;
    mac.update(media.canonical_payload().as_bytes());
    mac.verify_slice(&signature)
        .map_err(|_| MediaUrlVerificationError::InvalidSignature)
}

fn validate_media_url_fields(media: &SignedMediaUrl<'_>) -> Result<(), MediaUrlVerificationError> {
    if !media.path.starts_with("/media/v1/")
        || media.path.len() > 2048
        || !media.path.bytes().all(|byte| byte.is_ascii_graphic())
        || media.expires_at <= 0
    {
        return Err(MediaUrlVerificationError::InvalidField);
    }
    Ok(())
}

fn media_url_mac(secret: &SecretString) -> Result<HmacSha256, MediaUrlVerificationError> {
    if !(32..=256).contains(&secret.expose_secret().len()) {
        return Err(MediaUrlVerificationError::InvalidKey);
    }
    HmacSha256::new_from_slice(secret.expose_secret().as_bytes())
        .map_err(|_| MediaUrlVerificationError::InvalidKey)
}

fn valid_field(request: &SignedRequest<'_>) -> bool {
    !request.key_id.is_empty()
        && request.key_id.len() <= 128
        && request
            .key_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        && (16..=128).contains(&request.nonce.len())
        && request.nonce.bytes().all(|byte| byte.is_ascii_graphic())
        && !request.method.is_empty()
        && request.method.bytes().all(|byte| byte.is_ascii_uppercase())
        && request.path_and_query.starts_with('/')
        && request.path_and_query.len() <= 8192
        && request
            .path_and_query
            .bytes()
            .all(|byte| byte.is_ascii_graphic())
        && request.body_sha256.len() == 64
        && request
            .body_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use hmac::{Hmac, KeyInit as _, Mac};
    use secrecy::SecretString;
    use sha2::Sha256;

    use super::{
        MediaUrlVerificationError, SignedMediaUrl, SignedRequest, VerificationError, sha256_hex,
        sign_media_url, verify, verify_media_url,
    };

    fn signed_request<'a>(
        body_hash: &'a str,
        signature: &'a str,
        path: &'a str,
    ) -> SignedRequest<'a> {
        SignedRequest {
            key_id: "g5-primary",
            timestamp: 1_700_000_000,
            nonce: "0123456789abcdef0123456789abcdef",
            method: "POST",
            path_and_query: path,
            body_sha256: body_hash,
            signature,
        }
    }

    #[test]
    fn accepts_valid_signature_and_rejects_tamper() -> Result<(), Box<dyn std::error::Error>> {
        let secret = SecretString::from("0123456789abcdef0123456789abcdef".to_owned());
        let body_hash = sha256_hex(br#"{"ok":true}"#);
        let unsigned = signed_request(&body_hash, "", "/v1/uploads/intents");
        let mut mac = Hmac::<Sha256>::new_from_slice(b"0123456789abcdef0123456789abcdef")?;
        mac.update(unsigned.canonical_payload().as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        let request = signed_request(&body_hash, &signature, "/v1/uploads/intents");
        verify(&request, &secret, 1_700_000_100, 300)?;

        let tampered = signed_request(&body_hash, &signature, "/v1/uploads/other");
        assert_eq!(
            verify(&tampered, &secret, 1_700_000_100, 300),
            Err(VerificationError::InvalidSignature)
        );
        Ok(())
    }

    #[test]
    fn rejects_stale_request_before_signature_work() {
        let secret = SecretString::from("secret".to_owned());
        let body_hash = sha256_hex(b"");
        let request = signed_request(&body_hash, "invalid", "/health");
        assert_eq!(
            verify(&request, &secret, 1_700_001_000, 300),
            Err(VerificationError::StaleTimestamp)
        );
    }

    #[test]
    fn rejects_short_keys_and_noncanonical_hashes() {
        let short_secret = SecretString::from("too-short".to_owned());
        let body_hash = sha256_hex(b"");
        let request = signed_request(&body_hash, "invalid", "/health");
        assert_eq!(
            verify(&request, &short_secret, 1_700_000_000, 300),
            Err(VerificationError::InvalidKey)
        );

        let strong_secret = SecretString::from("0123456789abcdef0123456789abcdef".to_owned());
        let uppercase_hash = body_hash.to_uppercase();
        let noncanonical = signed_request(&uppercase_hash, "invalid", "/health");
        assert_eq!(
            verify(&noncanonical, &strong_secret, 1_700_000_000, 300),
            Err(VerificationError::InvalidField)
        );
    }

    #[test]
    fn media_url_signature_rejects_tamper_expiry_and_excessive_lifetime()
    -> Result<(), Box<dyn std::error::Error>> {
        let secret = SecretString::from("0123456789abcdef0123456789abcdef".to_owned());
        let path = "/media/v1/site-a/018f5a0c-7282-7a77-87dd-7a944e3b6f22/board-v1/thumbnail.jpg";
        let unsigned = SignedMediaUrl {
            path,
            expires_at: 1_700_000_300,
            signature: "",
        };
        let signature = sign_media_url(&unsigned, &secret)?;
        let signed = SignedMediaUrl {
            signature: &signature,
            ..unsigned
        };
        verify_media_url(&signed, &secret, 1_700_000_000, 300)?;

        let tampered = SignedMediaUrl {
            path: "/media/v1/site-a/018f5a0c-7282-7a77-87dd-7a944e3b6f22/other/thumbnail.jpg",
            ..signed
        };
        assert_eq!(
            verify_media_url(&tampered, &secret, 1_700_000_000, 300),
            Err(MediaUrlVerificationError::InvalidSignature)
        );
        assert_eq!(
            verify_media_url(&signed, &secret, 1_700_000_301, 300),
            Err(MediaUrlVerificationError::Expired)
        );
        assert_eq!(
            verify_media_url(&signed, &secret, 1_699_999_999, 300),
            Err(MediaUrlVerificationError::ExcessiveLifetime)
        );
        Ok(())
    }
}
