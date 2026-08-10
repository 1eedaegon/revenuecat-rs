//! The RevenueCat Test Store (`test_` API keys), mirroring
//! `SimulatedStoreBillingWrapper` (Android) and `SimulatedStore` (iOS):
//! products come from the Web Billing products endpoint and purchases are
//! fabricated locally — no native store involved. This is what makes desktop
//! and CI end-to-end testing (including the Tauri demo) possible.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::backend::Backend;
use crate::error::{Error, ErrorCode, Result};
use crate::identity::IdentityManager;
use crate::models::{Store, StoreProduct, StoreTransaction};

use super::StoreBilling;

pub struct SimulatedStoreBilling {
    backend: Arc<Backend>,
    identity: Arc<IdentityManager>,
}

impl SimulatedStoreBilling {
    pub(crate) fn new(backend: Arc<Backend>, identity: Arc<IdentityManager>) -> Self {
        Self { backend, identity }
    }
}

#[async_trait]
impl StoreBilling for SimulatedStoreBilling {
    async fn query_products(&self, product_ids: &[String]) -> Result<Vec<StoreProduct>> {
        if product_ids.is_empty() {
            return Ok(Vec::new());
        }
        let app_user_id = self.identity.current_app_user_id();
        let response = self
            .backend
            .get_web_billing_products(&app_user_id, product_ids)
            .await?;
        response
            .product_details
            .iter()
            .map(StoreProduct::try_from)
            .collect()
    }

    async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction> {
        let now = Utc::now();
        // Token format shared by purchases-js and purchases-android:
        // `test_${purchaseTimeMillis}_${UUID}` — the backend recognizes
        // test-store receipts by this prefix.
        let purchase_token = format!("test_{}_{}", now.timestamp_millis(), Uuid::new_v4());
        Ok(StoreTransaction {
            purchase_token: purchase_token.clone(),
            product_ids: vec![product.identifier.clone()],
            purchase_date: now,
            transaction_id: Some(purchase_token),
            store: Store::TestStore,
            price: Some(product.price.clone()),
        })
    }

    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
        // The simulated store keeps no local purchase state, matching
        // `SimulatedStoreBillingWrapper.queryPurchases` returning empty.
        Ok(Vec::new())
    }

    async fn finish_transaction(&self, _: &StoreTransaction, _: bool) -> Result<()> {
        // No-op: there is no store to acknowledge against.
        Ok(())
    }
}

/// Returned when a non-test API key is configured without a native store
/// bridge; kept here so the message lives next to the seam it points at.
pub(crate) fn missing_store_billing_error() -> Error {
    Error::new(
        ErrorCode::ConfigurationError,
        "This API key requires a native store. Provide ConfigurationBuilder::store_billing \
         with a StoreKit/Play Billing bridge, or use a `test_` (Test Store) API key.",
    )
}
