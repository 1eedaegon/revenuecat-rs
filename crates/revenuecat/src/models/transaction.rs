//! `StoreTransaction`: the store-agnostic result of a purchase, whose
//! `purchase_token` becomes the `fetch_token` posted to `/v1/receipts`.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::fmt;

use super::customer_info::Store;
use super::offerings::TargetingResponse;
use super::store_product::Price;

/// Store-specific product identity posted as `platform_product_ids` in the
/// current RevenueCat receipt payload.
///
/// Google subscriptions include the selected base plan and (when applicable)
/// offer. Other stores normally use only `product_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct PlatformProductId {
    product_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_plan_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offer_id: Option<String>,
}

impl PlatformProductId {
    /// Creates a platform identifier for stores without a separate plan.
    pub fn new(product_id: impl Into<String>) -> Self {
        Self {
            product_id: product_id.into(),
            base_plan_id: None,
            offer_id: None,
        }
    }

    /// Creates a Google Play base-plan identifier without an offer.
    pub fn google(product_id: impl Into<String>, base_plan_id: impl Into<String>) -> Self {
        Self {
            product_id: product_id.into(),
            base_plan_id: Some(base_plan_id.into()),
            offer_id: None,
        }
    }

    /// Creates a Google Play identifier for a specific subscription offer.
    pub fn google_with_offer(
        product_id: impl Into<String>,
        base_plan_id: impl Into<String>,
        offer_id: impl Into<String>,
    ) -> Self {
        Self {
            product_id: product_id.into(),
            base_plan_id: Some(base_plan_id.into()),
            offer_id: Some(offer_id.into()),
        }
    }

    pub fn product_id(&self) -> &str {
        &self.product_id
    }

    pub fn base_plan_id(&self) -> Option<&str> {
        self.base_plan_id.as_deref()
    }

    pub fn offer_id(&self) -> Option<&str> {
        self.offer_id.as_deref()
    }
}

/// Store selection requested by an Offering package.
///
/// On Google Play, `platform_product_plan_identifier` is the base-plan ID
/// selected in the RevenueCat dashboard. Legacy [`crate::StoreBilling`]
/// implementations may ignore it through the trait's provided fallback.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StorePurchaseOption {
    platform_product_plan_identifier: Option<String>,
}

/// Receipt attribution captured before an SDK-originated store purchase.
///
/// Native bridges can attach this context to [`StorePurchase`] when an
/// unfinished transaction is recovered after a process restart. This keeps
/// Offering, Placement, and Targeting attribution intact without changing the
/// public [`StoreTransaction`] shape used by existing 1.x adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct StoreReceiptContext {
    presented_offering_identifier: Option<String>,
    presented_placement_identifier: Option<String>,
    applied_targeting_rule: Option<TargetingResponse>,
    price: Option<Price>,
    normal_duration: Option<String>,
    sdk_originated: bool,
}

impl StoreReceiptContext {
    pub fn new(
        presented_offering_identifier: Option<String>,
        presented_placement_identifier: Option<String>,
        applied_targeting_rule: Option<TargetingResponse>,
        price: Option<Price>,
        normal_duration: Option<String>,
        sdk_originated: bool,
    ) -> Self {
        Self {
            presented_offering_identifier,
            presented_placement_identifier,
            applied_targeting_rule,
            price,
            normal_duration,
            sdk_originated,
        }
    }

    pub fn presented_offering_identifier(&self) -> Option<&str> {
        self.presented_offering_identifier.as_deref()
    }

    pub fn presented_placement_identifier(&self) -> Option<&str> {
        self.presented_placement_identifier.as_deref()
    }

    pub fn applied_targeting_rule(&self) -> Option<&TargetingResponse> {
        self.applied_targeting_rule.as_ref()
    }

    pub fn price(&self) -> Option<&Price> {
        self.price.as_ref()
    }

    pub fn normal_duration(&self) -> Option<&str> {
        self.normal_duration.as_deref()
    }

    pub fn sdk_originated(&self) -> bool {
        self.sdk_originated
    }
}

impl StorePurchaseOption {
    pub fn for_plan(plan_identifier: impl Into<String>) -> Self {
        Self {
            platform_product_plan_identifier: Some(plan_identifier.into()),
        }
    }

    pub fn platform_product_plan_identifier(&self) -> Option<&str> {
        self.platform_product_plan_identifier.as_deref()
    }
}

#[derive(Clone, Serialize)]
pub struct StoreTransaction {
    /// Store token proving the purchase. Simulated-store tokens look like
    /// `test_<epoch_ms>_<uuid>`, matching purchases-js / purchases-android.
    pub purchase_token: String,
    pub product_ids: Vec<String>,
    pub purchase_date: DateTime<Utc>,
    pub transaction_id: Option<String>,
    pub store: Store,
    /// Price paid, forwarded to the backend for revenue tracking.
    pub price: Option<Price>,
}

impl fmt::Debug for StoreTransaction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreTransaction")
            .field("purchase_token", &"[REDACTED]")
            .field("product_ids", &self.product_ids)
            .field("purchase_date", &self.purchase_date)
            .field("transaction_id", &self.transaction_id)
            .field("store", &self.store)
            .field("price", &self.price)
            .finish()
    }
}

/// A transaction plus the exact store product options used for its receipt.
///
/// This wrapper extends the native billing seam without adding fields to the
/// public [`StoreTransaction`] struct, so existing 1.x struct literals keep
/// compiling.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct StorePurchase {
    pub transaction: StoreTransaction,
    pub platform_product_ids: Vec<PlatformProductId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt_context: Option<StoreReceiptContext>,
}

impl StorePurchase {
    pub fn new(
        transaction: StoreTransaction,
        platform_product_ids: Vec<PlatformProductId>,
    ) -> Self {
        Self {
            transaction,
            platform_product_ids,
            receipt_context: None,
        }
    }

    /// Attaches attribution recovered by a native transaction journal.
    pub fn with_receipt_context(mut self, receipt_context: StoreReceiptContext) -> Self {
        self.receipt_context = Some(receipt_context);
        self
    }

    pub fn receipt_context(&self) -> Option<&StoreReceiptContext> {
        self.receipt_context.as_ref()
    }

    /// Adapts a legacy store transaction by retaining its product IDs without
    /// claiming base-plan or offer information the implementation did not
    /// provide.
    pub fn from_transaction(transaction: StoreTransaction) -> Self {
        let platform_product_ids = transaction
            .product_ids
            .iter()
            .cloned()
            .map(PlatformProductId::new)
            .collect();
        Self::new(transaction, platform_product_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transaction() -> StoreTransaction {
        StoreTransaction {
            purchase_token: "secret-jws-or-play-token".into(),
            product_ids: vec!["monthly".into()],
            purchase_date: DateTime::from_timestamp_millis(1_700_000_000_000).unwrap(),
            transaction_id: Some("42".into()),
            store: Store::AppStore,
            price: None,
        }
    }

    #[test]
    fn debug_redacts_purchase_token_through_transaction_and_purchase() {
        let transaction_debug = format!("{:?}", transaction());
        let purchase_debug = format!("{:?}", StorePurchase::from_transaction(transaction()));

        for debug in [transaction_debug, purchase_debug] {
            assert!(!debug.contains("secret-jws-or-play-token"));
            assert!(debug.contains("[REDACTED]"));
        }
    }

    #[test]
    fn serialization_retains_purchase_token_for_the_receipt_wire() {
        let serialized = serde_json::to_value(transaction()).unwrap();

        assert_eq!(serialized["purchase_token"], "secret-jws-or-play-token");
    }
}
