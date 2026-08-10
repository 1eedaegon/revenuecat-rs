//! # revenuecat
//!
//! Unofficial RevenueCat SDK for Rust, protocol-compatible with the official
//! [purchases-ios], [purchases-android], and [purchases-js] SDKs (all MIT).
//! The server-side contract — endpoints, headers, custom ETag caching, error
//! codes, and the `POST /v1/receipts` body — was extracted from those SDKs'
//! sources and test fixtures.
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
mod error;
mod http;
mod identity;
mod models;
mod purchases;

pub use backend::ReceiptRequest;
pub use billing::{SimulatedStoreBilling, StoreBilling};
pub use configuration::{
    ApiKeyKind, Configuration, ConfigurationBuilder, Platform, DEFAULT_API_BASE,
};
pub use error::{Error, ErrorCode, Result};
pub use identity::{generate_anonymous_id, is_anonymous_id, ANONYMOUS_ID_PREFIX};
pub use models::*;
pub use purchases::{CacheFetchPolicy, PurchaseResult, Purchases};
