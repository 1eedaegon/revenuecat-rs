//! Store abstraction, mirroring `BillingAbstract` (Android) and the
//! `StoreKitAbstractions` seam (iOS). Everything above this trait is pure
//! logic + HTTP; native stores (StoreKit / Play Billing, e.g. via a Tauri
//! mobile plugin) plug in below it.

pub(crate) mod simulated;

use async_trait::async_trait;

use crate::error::Result;
use crate::models::{StoreProduct, StoreTransaction};

pub use simulated::SimulatedStoreBilling;

#[async_trait]
pub trait StoreBilling: Send + Sync {
    /// Fetches store products (localized pricing) for the given identifiers.
    async fn query_products(&self, product_ids: &[String]) -> Result<Vec<StoreProduct>>;

    /// Runs the store purchase flow and returns the resulting transaction.
    /// The SDK then posts `transaction.purchase_token` to `/v1/receipts`.
    async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction>;

    /// Returns currently-owned transactions (used by restore).
    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>>;

    /// Acknowledges/consumes/finishes a transaction AFTER the backend has
    /// accepted the receipt — the same ordering the official SDKs enforce.
    ///
    /// `should_consume` is `true` only for consumable purchases: consume on
    /// Play Billing (so the product can be repurchased), plain `finish()` on
    /// StoreKit 2 (which has no client-side consume). With `false`,
    /// acknowledge on Play Billing — never consume a subscription token,
    /// that is a Play Billing developer error — and `finish()` on StoreKit.
    async fn finish_transaction(
        &self,
        transaction: &StoreTransaction,
        should_consume: bool,
    ) -> Result<()>;
}
