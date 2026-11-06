//! Store abstraction, mirroring `BillingAbstract` (Android) and the
//! `StoreKitAbstractions` seam (iOS). Everything above this trait is pure
//! logic + HTTP; native stores (StoreKit / Play Billing, e.g. via a Tauri
//! mobile plugin) plug in below it.

pub(crate) mod simulated;

use async_trait::async_trait;

use crate::error::Result;
use crate::models::{
    StoreProduct, StoreProductQuery, StoreProductResult, StorePurchase, StorePurchaseOption,
    StoreReceiptContext, StoreTransaction,
};

pub use simulated::SimulatedStoreBilling;

#[async_trait]
pub trait StoreBilling: Send + Sync {
    /// Returns whether the native store is currently using a sandbox
    /// environment, when the platform can determine it.
    ///
    /// The provided `None` result preserves source compatibility for 1.x
    /// store adapters and means the HTTP layer should apply its normal API-key
    /// fallback instead of asserting either environment.
    async fn is_sandbox_environment(&self) -> Result<Option<bool>> {
        Ok(None)
    }

    /// Fetches store products (localized pricing) for the given identifiers.
    async fn query_products(&self, product_ids: &[String]) -> Result<Vec<StoreProduct>>;

    /// Fetches localized products for Offering product/base-plan pairs.
    ///
    /// The provided implementation deduplicates product IDs and delegates to
    /// [`StoreBilling::query_products`], returning plan-less fallback results.
    /// Google Play bridges should override this to return one
    /// [`StoreProductResult`] per actual base plan so packages sharing a
    /// product ID retain the correct price and period.
    async fn query_products_with_options(
        &self,
        queries: &[StoreProductQuery],
    ) -> Result<Vec<StoreProductResult>> {
        let mut product_ids = queries
            .iter()
            .map(|query| query.product_id().to_owned())
            .collect::<Vec<_>>();
        product_ids.sort();
        product_ids.dedup();
        Ok(self
            .query_products(&product_ids)
            .await?
            .into_iter()
            .map(StoreProductResult::new)
            .collect())
    }

    /// Runs the store purchase flow and returns the resulting transaction.
    /// The SDK then posts `transaction.purchase_token` to `/v1/receipts`.
    async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction>;

    /// Purchases a product using store-specific Offering selection metadata
    /// and returns the exact platform product IDs used by the store.
    ///
    /// This provided method preserves source compatibility for existing 1.x
    /// implementations: it delegates to [`StoreBilling::purchase`] and emits
    /// product-only identifiers. Google Play bridges should override it to
    /// select `option.platform_product_plan_identifier()` and return the
    /// actual base-plan/offer IDs in [`StorePurchase`].
    async fn purchase_with_option(
        &self,
        product: &StoreProduct,
        _option: Option<&StorePurchaseOption>,
    ) -> Result<StorePurchase> {
        self.purchase(product)
            .await
            .map(StorePurchase::from_transaction)
    }

    /// Purchases a product while exposing the current RevenueCat app user ID
    /// to native stores that support account attribution.
    ///
    /// The provided implementation preserves source compatibility for 1.x
    /// adapters by delegating to [`StoreBilling::purchase_with_option`].
    /// StoreKit 2 bridges can override this method and use a UUID-shaped user
    /// ID as the purchase's `appAccountToken`.
    async fn purchase_with_option_for_user(
        &self,
        product: &StoreProduct,
        option: Option<&StorePurchaseOption>,
        _app_user_id: &str,
    ) -> Result<StorePurchase> {
        self.purchase_with_option(product, option).await
    }

    /// Purchases while forwarding receipt attribution to native storage.
    ///
    /// The provided implementation keeps existing 1.x billing adapters source
    /// compatible. StoreKit/Play bridges may override this to journal the
    /// context against the concrete transaction before returning it.
    async fn purchase_with_receipt_context_for_user(
        &self,
        product: &StoreProduct,
        option: Option<&StorePurchaseOption>,
        app_user_id: &str,
        _receipt_context: &StoreReceiptContext,
    ) -> Result<StorePurchase> {
        self.purchase_with_option_for_user(product, option, app_user_id)
            .await
    }

    /// Returns currently-owned transactions (used by restore).
    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>>;

    /// Returns owned transactions with their exact platform product IDs.
    /// Existing implementations inherit a product-only adapter; native
    /// bridges can override this to retain Google base-plan and offer IDs on
    /// restore and background synchronization paths.
    async fn query_purchases_with_context(&self) -> Result<Vec<StorePurchase>> {
        Ok(self
            .query_purchases()
            .await?
            .into_iter()
            .map(StorePurchase::from_transaction)
            .collect())
    }

    /// Returns only transactions that still need backend synchronization.
    ///
    /// Existing 1.x implementations inherit the safe compatibility fallback
    /// of querying every owned purchase. Native bridges should override this:
    /// StoreKit can return `Transaction.unfinished`, while Play can exclude
    /// already-acknowledged purchases. This avoids reposting the complete
    /// active-entitlement history on every store queue update.
    async fn query_pending_purchases_with_context(&self) -> Result<Vec<StorePurchase>> {
        self.query_purchases_with_context().await
    }

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

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::models::{
        PlatformProductId, Price, ProductType, Store, StoreProductQuery, StorePurchase,
        StorePurchaseOption,
    };

    struct LegacyBilling;

    #[async_trait]
    impl StoreBilling for LegacyBilling {
        async fn query_products(&self, product_ids: &[String]) -> Result<Vec<StoreProduct>> {
            Ok(product_ids
                .iter()
                .filter(|id| id.as_str() == "monthly")
                .map(|_| product())
                .collect())
        }

        async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction> {
            Ok(StoreTransaction {
                purchase_token: "token".into(),
                product_ids: vec![product.identifier.clone()],
                purchase_date: Utc::now(),
                transaction_id: None,
                store: Store::PlayStore,
                price: Some(product.price.clone()),
            })
        }

        async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
            Ok(Vec::new())
        }

        async fn finish_transaction(&self, _: &StoreTransaction, _: bool) -> Result<()> {
            Ok(())
        }
    }

    fn product() -> StoreProduct {
        StoreProduct {
            identifier: "monthly".into(),
            product_type: ProductType::Subscription,
            title: "Monthly".into(),
            description: None,
            price: Price {
                amount_micros: 1_000_000,
                currency: "USD".into(),
            },
            subscription_period: Some("P1M".into()),
            trial: None,
            intro_price: None,
        }
    }

    #[tokio::test]
    async fn provided_purchase_with_option_keeps_legacy_implementors_working() {
        let option = StorePurchaseOption::for_plan("monthly-base");

        let purchase = LegacyBilling
            .purchase_with_option(&product(), Some(&option))
            .await
            .unwrap();

        assert_eq!(purchase.transaction.product_ids, vec!["monthly"]);
        assert_eq!(
            purchase.platform_product_ids,
            vec![PlatformProductId::new("monthly")]
        );
    }

    #[tokio::test]
    async fn provided_user_aware_purchase_keeps_legacy_implementors_working() {
        let option = StorePurchaseOption::for_plan("monthly-base");

        let purchase = LegacyBilling
            .purchase_with_option_for_user(&product(), Some(&option), "current-user")
            .await
            .unwrap();

        assert_eq!(purchase.transaction.product_ids, vec!["monthly"]);
        assert_eq!(
            purchase.platform_product_ids,
            vec![PlatformProductId::new("monthly")]
        );
    }

    #[tokio::test]
    async fn provided_receipt_context_purchase_keeps_legacy_implementors_working() {
        let option = StorePurchaseOption::for_plan("monthly-base");
        let receipt_context = StoreReceiptContext::new(
            Some("default".into()),
            None,
            None,
            Some(product().price),
            Some("P1M".into()),
            true,
        );

        let purchase = LegacyBilling
            .purchase_with_receipt_context_for_user(
                &product(),
                Some(&option),
                "current-user",
                &receipt_context,
            )
            .await
            .unwrap();

        assert_eq!(purchase.transaction.product_ids, vec!["monthly"]);
        assert_eq!(purchase.receipt_context(), None);
    }

    #[tokio::test]
    async fn provided_product_query_keeps_legacy_implementors_working() {
        let queries = vec![
            StoreProductQuery::for_plan("monthly", "monthly-base"),
            StoreProductQuery::for_plan("monthly", "annual-base"),
        ];

        let products = LegacyBilling
            .query_products_with_options(&queries)
            .await
            .unwrap();

        assert_eq!(products.len(), 1, "legacy query deduplicates product IDs");
        assert_eq!(products[0].product().identifier, "monthly");
        assert_eq!(products[0].platform_product_plan_identifier(), None);
    }

    #[tokio::test]
    async fn provided_sandbox_query_keeps_legacy_implementors_working() {
        assert_eq!(LegacyBilling.is_sandbox_environment().await.unwrap(), None);
    }

    #[test]
    fn richer_store_purchase_can_retain_google_plan_and_offer() {
        let transaction = StoreTransaction {
            purchase_token: "token".into(),
            product_ids: vec!["monthly".into()],
            purchase_date: Utc::now(),
            transaction_id: None,
            store: Store::PlayStore,
            price: None,
        };

        let purchase = StorePurchase::new(
            transaction,
            vec![PlatformProductId::google_with_offer(
                "monthly",
                "monthly-base",
                "intro-offer",
            )],
        );

        assert_eq!(
            purchase.platform_product_ids[0].base_plan_id(),
            Some("monthly-base")
        );
        assert_eq!(
            purchase.platform_product_ids[0].offer_id(),
            Some("intro-offer")
        );
        assert_eq!(
            PlatformProductId::google("monthly", "monthly-base").offer_id(),
            None
        );
    }
}
