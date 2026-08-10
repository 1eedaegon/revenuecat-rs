//! The X-Signature wire format:
//!
//! ```text
//! base64( intermediate_key(32) || intermediate_key_expiration(4, LE days since 1970)
//!         || intermediate_key_signature(64) || salt(16) || payload_signature(64) )
//! ```
//!
//! The root key signs `expiration || intermediate_key`; the intermediate key
//! signs `salt || api_key || nonce || url_path || post_params_hash ||
//! request_time || etag || body` (missing components are empty). Layout and
//! message composition follow purchases-android `Signature.kt` /
//! `SigningManager.kt` exactly.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

/// RevenueCat's production root Ed25519 public key
/// (`DefaultSignatureVerifier.DEFAULT_PUBLIC_KEY` / `Signing.publicKey`).
pub const ROOT_PUBLIC_KEY_B64: &str = "UC1upXWg5QVmyOSwozp755xLqquBKjjU+di6U8QhMlM=";

pub const SIGNATURE_LEN: usize = 180;

const INTERMEDIATE_KEY_LEN: usize = 32;
const EXPIRATION_LEN: usize = 4;
const KEY_SIGNATURE_LEN: usize = 64;
const SALT_LEN: usize = 16;
const PAYLOAD_LEN: usize = 64;
const NONCE_LEN: usize = 12;
const MS_PER_DAY: i64 = 86_400_000;

/// Parsed X-Signature blob (fixed 180-byte layout).
#[derive(Debug, Clone)]
pub struct SignatureBlob {
    pub intermediate_key: [u8; INTERMEDIATE_KEY_LEN],
    pub intermediate_key_expiration: [u8; EXPIRATION_LEN],
    pub intermediate_key_signature: [u8; KEY_SIGNATURE_LEN],
    pub salt: [u8; SALT_LEN],
    pub payload: [u8; PAYLOAD_LEN],
}

impl SignatureBlob {
    pub fn parse_base64(header_value: &str) -> Result<Self, String> {
        let bytes = BASE64
            .decode(header_value.trim())
            .map_err(|e| format!("X-Signature is not valid base64: {e}"))?;
        if bytes.len() != SIGNATURE_LEN {
            return Err(format!(
                "X-Signature must decode to {SIGNATURE_LEN} bytes, got {}",
                bytes.len()
            ));
        }
        let mut offset = 0;
        let mut take = |len: usize| {
            let slice = &bytes[offset..offset + len];
            offset += len;
            slice.to_vec()
        };
        Ok(Self {
            intermediate_key: take(INTERMEDIATE_KEY_LEN).try_into().expect("sized"),
            intermediate_key_expiration: take(EXPIRATION_LEN).try_into().expect("sized"),
            intermediate_key_signature: take(KEY_SIGNATURE_LEN).try_into().expect("sized"),
            salt: take(SALT_LEN).try_into().expect("sized"),
            payload: take(PAYLOAD_LEN).try_into().expect("sized"),
        })
    }
}

/// Everything that goes into the payload signed message for one response.
#[derive(Debug, Default)]
pub struct VerifyParams<'a> {
    /// Base64 X-Nonce value the REQUEST carried, if any.
    pub nonce_b64: Option<&'a str>,
    /// Request path with leading slash and no query (`/v1/subscribers/x`).
    pub url_path: &'a str,
    /// The X-Post-Params-Hash header value the request carried, if any.
    pub post_params_hash: Option<&'a str>,
    /// The X-RevenueCat-Request-Time response header value, verbatim.
    pub request_time: &'a str,
    /// The X-RevenueCat-ETag response header value, if any.
    pub etag: Option<&'a str>,
    /// Response body bytes (empty for 304).
    pub body: &'a [u8],
}

#[derive(Debug)]
pub struct SignatureVerifier {
    root_key: VerifyingKey,
    api_key: String,
}

impl SignatureVerifier {
    pub fn new(root_key_b64: &str, api_key: &str) -> Result<Self, String> {
        let bytes: [u8; 32] = BASE64
            .decode(root_key_b64.trim())
            .map_err(|e| format!("root key is not valid base64: {e}"))?
            .try_into()
            .map_err(|_| "root key must be 32 bytes".to_owned())?;
        let root_key = VerifyingKey::from_bytes(&bytes)
            .map_err(|e| format!("root key is not a valid Ed25519 key: {e}"))?;
        Ok(Self {
            root_key,
            api_key: api_key.to_owned(),
        })
    }

    /// Full verification: signature parse -> intermediate-key chain ->
    /// payload signature over the composed message. `Err` carries the reason
    /// (surfaced as log/underlying error, mapping to `VerificationResult::Failed`).
    pub fn verify(&self, signature_header: &str, params: &VerifyParams<'_>) -> Result<(), String> {
        let blob = SignatureBlob::parse_base64(signature_header)?;
        self.verify_intermediate_key(&blob)?;

        let intermediate = VerifyingKey::from_bytes(&blob.intermediate_key)
            .map_err(|e| format!("intermediate key invalid: {e}"))?;
        let message = self.signed_message(&blob, params)?;
        intermediate
            .verify(&message, &Signature::from_bytes(&blob.payload))
            .map_err(|_| "payload signature does not match".to_owned())
    }

    /// Root signature covers `expiration || intermediate_key` (that order),
    /// then the expiration (LE days since 1970) must be in the future.
    fn verify_intermediate_key(&self, blob: &SignatureBlob) -> Result<(), String> {
        let mut message = Vec::with_capacity(EXPIRATION_LEN + INTERMEDIATE_KEY_LEN);
        message.extend_from_slice(&blob.intermediate_key_expiration);
        message.extend_from_slice(&blob.intermediate_key);
        self.root_key
            .verify(
                &message,
                &Signature::from_bytes(&blob.intermediate_key_signature),
            )
            .map_err(|_| "intermediate key signature does not match root key".to_owned())?;

        let days = i32::from_le_bytes(blob.intermediate_key_expiration);
        if days <= 0 {
            return Err("intermediate key expiration is invalid".to_owned());
        }
        let expires_ms = i64::from(days) * MS_PER_DAY;
        let now_ms = chrono::Utc::now().timestamp_millis();
        if expires_ms < now_ms {
            return Err(format!("intermediate key expired at day {days} since 1970"));
        }
        Ok(())
    }

    /// `salt + apiKey + nonce + urlPath + postParamsHash + requestTime +
    /// eTag + body`, mirroring `Parameters.toSignatureToVerify()`.
    fn signed_message(
        &self,
        blob: &SignatureBlob,
        params: &VerifyParams<'_>,
    ) -> Result<Vec<u8>, String> {
        let nonce_bytes = match params.nonce_b64 {
            Some(nonce) => BASE64
                .decode(nonce.trim())
                .map_err(|e| format!("nonce is not valid base64: {e}"))?,
            None => Vec::new(),
        };
        let mut message = Vec::new();
        message.extend_from_slice(&blob.salt);
        message.extend_from_slice(self.api_key.as_bytes());
        message.extend_from_slice(&nonce_bytes);
        message.extend_from_slice(params.url_path.as_bytes());
        if let Some(hash) = params.post_params_hash {
            message.extend_from_slice(hash.as_bytes());
        }
        message.extend_from_slice(params.request_time.as_bytes());
        if let Some(etag) = params.etag {
            message.extend_from_slice(etag.as_bytes());
        }
        message.extend_from_slice(params.body);
        Ok(message)
    }
}

/// 12 random bytes, base64-encoded — the X-Nonce request header value.
pub fn generate_nonce() -> String {
    let mut bytes = [0u8; NONCE_LEN];
    rand::fill(&mut bytes);
    BASE64.encode(bytes)
}

/// `key1,key2:sha256:<hex>` where the hex is SHA-256 over the field VALUES
/// joined by a single 0x00 byte between them (no leading/trailing separator).
pub fn post_params_hash(fields: &[(&str, &str)]) -> String {
    let mut hasher = Sha256::new();
    for (index, (_, value)) in fields.iter().enumerate() {
        if index > 0 {
            hasher.update([0u8]);
        }
        hasher.update(value.as_bytes());
    }
    let hex = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let keys = fields
        .iter()
        .map(|(key, _)| *key)
        .collect::<Vec<_>>()
        .join(",");
    format!("{keys}:sha256:{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn keypair(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    /// Builds a valid signed blob + header the way the backend would.
    fn sign_response(
        root: &SigningKey,
        expiration_days: i32,
        api_key: &str,
        params: &VerifyParams<'_>,
    ) -> String {
        let intermediate = keypair(7);
        let expiration = expiration_days.to_le_bytes();
        let mut key_message = Vec::new();
        key_message.extend_from_slice(&expiration);
        key_message.extend_from_slice(intermediate.verifying_key().as_bytes());
        let key_signature = root.sign(&key_message);

        let salt = [9u8; 16];
        let mut message = Vec::new();
        message.extend_from_slice(&salt);
        message.extend_from_slice(api_key.as_bytes());
        if let Some(nonce) = params.nonce_b64 {
            message.extend_from_slice(&BASE64.decode(nonce).unwrap());
        }
        message.extend_from_slice(params.url_path.as_bytes());
        if let Some(hash) = params.post_params_hash {
            message.extend_from_slice(hash.as_bytes());
        }
        message.extend_from_slice(params.request_time.as_bytes());
        if let Some(etag) = params.etag {
            message.extend_from_slice(etag.as_bytes());
        }
        message.extend_from_slice(params.body);
        let payload = intermediate.sign(&message);

        let mut blob = Vec::with_capacity(SIGNATURE_LEN);
        blob.extend_from_slice(intermediate.verifying_key().as_bytes());
        blob.extend_from_slice(&expiration);
        blob.extend_from_slice(&key_signature.to_bytes());
        blob.extend_from_slice(&salt);
        blob.extend_from_slice(&payload.to_bytes());
        BASE64.encode(blob)
    }

    fn verifier_for(root: &SigningKey, api_key: &str) -> SignatureVerifier {
        let root_b64 = BASE64.encode(root.verifying_key().as_bytes());
        SignatureVerifier::new(&root_b64, api_key).unwrap()
    }

    fn future_days() -> i32 {
        (chrono::Utc::now().timestamp_millis() / MS_PER_DAY + 30) as i32
    }

    #[test]
    fn verifies_a_correctly_signed_response() {
        // Arrange
        let root = keypair(1);
        let params = VerifyParams {
            nonce_b64: Some("AAECAwQFBgcICQoL"),
            url_path: "/v1/subscribers/gon",
            post_params_hash: None,
            request_time: "1786371532643",
            etag: Some("abc123"),
            body: br#"{"subscriber":{}}"#,
        };
        let header = sign_response(&root, future_days(), "test_key", &params);

        // Act / Assert
        verifier_for(&root, "test_key")
            .verify(&header, &params)
            .unwrap();
    }

    #[test]
    fn rejects_tampered_body_and_wrong_api_key() {
        let root = keypair(1);
        let params = VerifyParams {
            url_path: "/v1/subscribers/gon",
            request_time: "1786371532643",
            body: b"original",
            ..Default::default()
        };
        let header = sign_response(&root, future_days(), "test_key", &params);
        let verifier = verifier_for(&root, "test_key");

        let tampered = VerifyParams {
            body: b"tampered",
            ..VerifyParams { ..params }
        };
        assert!(verifier.verify(&header, &tampered).is_err());

        let wrong_key = verifier_for(&root, "other_key");
        let original = VerifyParams {
            url_path: "/v1/subscribers/gon",
            request_time: "1786371532643",
            body: b"original",
            ..Default::default()
        };
        assert!(wrong_key.verify(&header, &original).is_err());
    }

    #[test]
    fn rejects_expired_and_invalid_expiration() {
        let root = keypair(1);
        let params = VerifyParams {
            url_path: "/x",
            request_time: "1",
            body: b"b",
            ..Default::default()
        };
        let verifier = verifier_for(&root, "k");

        let expired = sign_response(&root, 10, "k", &params); // day 10 of 1970
        assert!(verifier
            .verify(&expired, &params)
            .unwrap_err()
            .contains("expired"));

        let invalid = sign_response(&root, 0, "k", &params);
        assert!(verifier
            .verify(&invalid, &params)
            .unwrap_err()
            .contains("invalid"));
    }

    #[test]
    fn rejects_intermediate_key_signed_by_wrong_root() {
        let root = keypair(1);
        let other_root = keypair(2);
        let params = VerifyParams {
            url_path: "/x",
            request_time: "1",
            body: b"b",
            ..Default::default()
        };
        let header = sign_response(&other_root, future_days(), "k", &params);

        let error = verifier_for(&root, "k")
            .verify(&header, &params)
            .unwrap_err();
        assert!(error.contains("root key"));
    }

    #[test]
    fn rejects_wrong_blob_size() {
        assert!(SignatureBlob::parse_base64(&BASE64.encode([0u8; 179])).is_err());
        assert!(SignatureBlob::parse_base64("!!!not-base64!!!").is_err());
    }

    #[test]
    fn post_params_hash_matches_android_format() {
        // Arrange: sha256("gon" || 0x00 || "test_token") computed manually.
        let mut hasher = Sha256::new();
        hasher.update(b"gon");
        hasher.update([0u8]);
        hasher.update(b"test_token");
        let expected_hex = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();

        // Act
        let header = post_params_hash(&[("app_user_id", "gon"), ("fetch_token", "test_token")]);

        // Assert
        assert_eq!(
            header,
            format!("app_user_id,fetch_token:sha256:{expected_hex}")
        );
    }

    #[test]
    fn nonce_is_twelve_random_bytes_base64() {
        let nonce = generate_nonce();
        assert_eq!(BASE64.decode(&nonce).unwrap().len(), 12);
        assert_ne!(generate_nonce(), nonce);
    }

    #[test]
    fn production_root_key_parses() {
        SignatureVerifier::new(ROOT_PUBLIC_KEY_B64, "any").unwrap();
    }
}
