//! Trusted Entitlements response signing for the mock backend: a fixed test
//! root key signs a per-server intermediate key, which signs each response
//! exactly like the real backend (`salt || api_key || nonce || path ||
//! post_params_hash || request_time || etag || body`).

use std::sync::atomic::{AtomicBool, Ordering};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

const MS_PER_DAY: i64 = 86_400_000;
/// Deterministic test root key seed — NOT a secret; tests configure the SDK
/// with the matching public key via `test_root_public_key_b64()`.
const TEST_ROOT_SEED: [u8; 32] = [42u8; 32];

/// Base64 Ed25519 public key the SDK should trust when talking to this mock
/// (`ConfigurationBuilder::verification_root_key`).
pub fn test_root_public_key_b64() -> String {
    BASE64.encode(
        SigningKey::from_bytes(&TEST_ROOT_SEED)
            .verifying_key()
            .as_bytes(),
    )
}

#[derive(Debug)]
pub struct ResponseSigner {
    intermediate: SigningKey,
    /// `intermediate_key(32) || expiration(4 LE) || root_signature(64)` —
    /// the static prefix of every 180-byte signature blob.
    blob_prefix: Vec<u8>,
    /// When set, the payload signature is corrupted — for testing the SDK's
    /// failure paths.
    tamper: AtomicBool,
}

impl ResponseSigner {
    pub fn new(tamper: bool) -> Self {
        let root = SigningKey::from_bytes(&TEST_ROOT_SEED);
        let mut seed = [0u8; 32];
        rand::fill(&mut seed);
        let intermediate = SigningKey::from_bytes(&seed);

        let days = (chrono::Utc::now().timestamp_millis() / MS_PER_DAY + 30) as i32;
        let expiration = days.to_le_bytes();
        let mut key_message = Vec::with_capacity(4 + 32);
        key_message.extend_from_slice(&expiration);
        key_message.extend_from_slice(intermediate.verifying_key().as_bytes());
        let root_signature = root.sign(&key_message);

        let mut blob_prefix = Vec::with_capacity(32 + 4 + 64);
        blob_prefix.extend_from_slice(intermediate.verifying_key().as_bytes());
        blob_prefix.extend_from_slice(&expiration);
        blob_prefix.extend_from_slice(&root_signature.to_bytes());

        Self {
            intermediate,
            blob_prefix,
            tamper: AtomicBool::new(tamper),
        }
    }

    pub fn set_tamper(&self, tamper: bool) {
        self.tamper.store(tamper, Ordering::SeqCst);
    }

    /// Produces the base64 `X-Signature` header value for one response.
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        &self,
        api_key: &str,
        nonce_b64: Option<&str>,
        url_path: &str,
        post_params_hash: Option<&str>,
        request_time: &str,
        etag: Option<&str>,
        body: &[u8],
    ) -> String {
        let mut salt = [0u8; 16];
        rand::fill(&mut salt);

        let mut message = Vec::new();
        message.extend_from_slice(&salt);
        message.extend_from_slice(api_key.as_bytes());
        if let Some(nonce) = nonce_b64 {
            if let Ok(bytes) = BASE64.decode(nonce.trim()) {
                message.extend_from_slice(&bytes);
            }
        }
        message.extend_from_slice(url_path.as_bytes());
        if let Some(hash) = post_params_hash {
            message.extend_from_slice(hash.as_bytes());
        }
        message.extend_from_slice(request_time.as_bytes());
        if let Some(etag) = etag {
            message.extend_from_slice(etag.as_bytes());
        }
        message.extend_from_slice(body);

        let mut payload = self.intermediate.sign(&message).to_bytes();
        if self.tamper.load(Ordering::SeqCst) {
            payload[0] ^= 0xff;
        }

        let mut blob = Vec::with_capacity(180);
        blob.extend_from_slice(&self.blob_prefix);
        blob.extend_from_slice(&salt);
        blob.extend_from_slice(&payload);
        BASE64.encode(blob)
    }
}

/// `key1,key2:sha256:<hex>` over values joined by 0x00 — used to VALIDATE the
/// `X-Post-Params-Hash` header clients send.
pub fn expected_post_params_hash(fields: &[(&str, &str)]) -> String {
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
