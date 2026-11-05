//! # revenuecat-rs
//!
//! RevenueCat SDK for Rust, implemented as an independent direct port of
//! behavior in the official [purchases-ios], [purchases-android], and
//! [purchases-js] SDKs (all MIT). Compatibility is tracked per feature across
//! the backend wire contract and native purchase lifecycle; this crate does
//! not link those SDKs or claim parity with their UI surfaces.
//!
//! ```no_run
//! use revenuecat::{CacheFetchPolicy, Configuration, Purchases};
//!
//! # async fn run() -> revenuecat::Result<()> {
//! let purchases = Purchases::configure(
//!     Configuration::builder("test_your_api_key")
//!         .app_user_id("my-user")
//!         .build()?,
//! )?;
//!
//! let offerings = purchases.get_offerings().await?;
//! if let Some(package) = offerings.current().and_then(|o| o.monthly()) {
//!     let result = purchases.purchase_package(package).await?;
//!     assert!(result.customer_info.entitlements.is_active("pro"));
//! }
//!
//! let info = purchases.get_customer_info(CacheFetchPolicy::default()).await?;
//! println!("active: {:?}", info.active_subscriptions());
//! # Ok(())
//! # }
//! ```
//!
//! With a `test_` (Test Store) API key the built-in [`SimulatedStoreBilling`]
//! is used and no native store is required — purchases are simulated
//! end-to-end against the backend, which is how the Tauri demo and the
//! integration tests run on desktop. For real stores, implement
//! [`StoreBilling`] over StoreKit / Play Billing (e.g. from a Tauri mobile
//! plugin) and pass it via [`ConfigurationBuilder::store_billing`].
//!
//! [purchases-ios]: https://github.com/RevenueCat/purchases-ios
//! [purchases-android]: https://github.com/RevenueCat/purchases-android
//! [purchases-js]: https://github.com/RevenueCat/purchases-js

mod backend;
mod billing;
mod cache;
mod configuration;
mod diagnostics;
mod error;
mod http;
mod identity;
mod models;
mod purchases;
mod security;

pub use backend::ReceiptRequest;
pub use billing::{SimulatedStoreBilling, StoreBilling};
pub use configuration::{
    ApiKeyKind, Configuration, ConfigurationBuilder, EntitlementVerificationMode, Platform,
    DEFAULT_API_BASE,
};
pub use error::{Error, ErrorCode, Result};
pub use identity::{
    generate_anonymous_id, is_anonymous_id, ANONYMOUS_ID_PREFIX, MAX_APP_USER_ID_BYTES,
};
pub use models::*;
pub use purchases::{CacheFetchPolicy, PurchaseResult, Purchases};
pub use security::ROOT_PUBLIC_KEY_B64;
