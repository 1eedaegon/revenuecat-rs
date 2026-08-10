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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::configuration::Configuration;
    use crate::http::HttpClient;

    fn billing() -> SimulatedStoreBilling {
        let config = Configuration::builder("test_unit_key").build().unwrap();
        let diagnostics = Arc::new(crate::diagnostics::DiagnosticsTracker::new(false));
        let backend = Arc::new(Backend::new(HttpClient::new(&config, diagnostics).unwrap()));
        let identity = Arc::new(IdentityManager::new(Some("unit".into())));
        SimulatedStoreBilling::new(backend, identity)
    }

    fn product() -> StoreProduct {
        StoreProduct {
            identifier: "monthly".into(),
            product_type: crate::models::ProductType::Subscription,
            title: "Monthly".into(),
            description: None,
            price: crate::models::Price {
                amount_micros: 3_000_000,
                currency: "USD".into(),
            },
            subscription_period: Some("P1M".into()),
            trial: None,
            intro_price: None,
        }
    }

    #[tokio::test]
    async fn purchase_fabricates_a_test_store_token() {
        // Act
        let transaction = purchase_once().await;

        // Assert: `test_<epoch_ms>_<uuid>` — the prefix the backend uses to
        // recognize simulated receipts.
        let token = &transaction.purchase_token;
        assert!(token.starts_with("test_"));
        let parts: Vec<&str> = token.splitn(3, '_').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts[1].parse::<i64>().unwrap() > 0, "millis timestamp");
        assert_eq!(parts[2].len(), 36, "hyphenated uuid");
        assert_eq!(transaction.store, Store::TestStore);
        assert_eq!(transaction.product_ids, vec!["monthly".to_owned()]);
        assert_eq!(transaction.price.as_ref().unwrap().amount_micros, 3_000_000);
        assert_eq!(transaction.transaction_id.as_deref(), Some(token.as_str()));
    }

    async fn purchase_once() -> StoreTransaction {
        billing().purchase(&product()).await.unwrap()
    }

    #[tokio::test]
    async fn tokens_are_unique_per_purchase() {
        let billing = billing();
        let first = billing.purchase(&product()).await.unwrap();
        let second = billing.purchase(&product()).await.unwrap();
        assert_ne!(first.purchase_token, second.purchase_token);
    }

    #[tokio::test]
    async fn query_products_with_no_ids_skips_the_backend() {
        // No mock server is running, so a network call would error.
        let products = billing().query_products(&[]).await.unwrap();
        assert!(products.is_empty());
    }

    #[tokio::test]
    async fn query_purchases_is_empty_and_finish_is_a_no_op() {
        let billing = billing();
        assert!(billing.query_purchases().await.unwrap().is_empty());
        let transaction = billing.purchase(&product()).await.unwrap();
        billing
            .finish_transaction(&transaction, true)
            .await
            .unwrap();
    }
}
