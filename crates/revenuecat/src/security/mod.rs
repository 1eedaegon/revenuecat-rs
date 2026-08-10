//! Trusted Entitlements response-signature verification (Ed25519),
//! byte-compatible with `SigningManager`/`Signature` in purchases-android and
//! `Signing` in purchases-ios.

mod signature;

pub use signature::{
    generate_nonce, post_params_hash, SignatureVerifier, VerifyParams, ROOT_PUBLIC_KEY_B64,
};
