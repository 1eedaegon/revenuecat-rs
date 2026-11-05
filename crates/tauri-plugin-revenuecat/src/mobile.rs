//! `StoreBilling` implementation calling the native shim over the Tauri
//! mobile plugin channel. Store calls that show UI (purchase) block until
//! the user acts, so every call runs on the blocking pool.

use async_trait::async_trait;
use revenuecat::{
    Error, ErrorCode, Result, StoreBilling, StoreProduct, StoreProductQuery, StoreProductResult,
    StorePurchase, StorePurchaseOption, StoreReceiptContext, StoreTransaction,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tauri::plugin::PluginHandle;
use tauri::Runtime;

use crate::models::{
    FinishTransactionArgs, NativeProductsResponse, NativePurchaseResponse, NativePurchasesResponse,
    NativeSandboxEnvironmentResponse, PurchaseArgs, QueryProductsArgs,
};

pub struct MobileStoreBilling<R: Runtime> {
    handle: PluginHandle<R>,
}

impl<R: Runtime> MobileStoreBilling<R> {
    pub fn new(handle: PluginHandle<R>) -> Self {
        Self { handle }
    }

    async fn run<A, T>(&self, method: &'static str, args: A) -> Result<T>
    where
        A: Serialize + Send + 'static,
        T: DeserializeOwned + Send + 'static,
    {
        let handle = self.handle.clone();
        tauri::async_runtime::spawn_blocking(move || handle.run_mobile_plugin(method, args))
            .await
            .map_err(|e| Error::with_underlying(ErrorCode::UnknownError, e.to_string()))?
            .map_err(|e| {
                let message = e.to_string();
                // The shims reject with "cancelled" when the user backs out.
                if message.contains("cancelled") {
                    Error::new(ErrorCode::PurchaseCancelledError, "Purchase was cancelled.")
                } else if message.contains("payment pending") {
                    Error::new(ErrorCode::PaymentPendingError, "The payment is pending.")
                } else {
                    Error::with_underlying(ErrorCode::StoreProblemError, message)
                }
            })
    }

    async fn query_native_products(
        &self,
        args: QueryProductsArgs,
    ) -> Result<NativeProductsResponse> {
        self.run("queryProducts", args).await
    }

    async fn purchase_native(
        &self,
        product: &StoreProduct,
        option: Option<&StorePurchaseOption>,
        app_user_id: Option<&str>,
        receipt_context: Option<&StoreReceiptContext>,
    ) -> Result<StorePurchase> {
        let base_plan_id = option.and_then(|value| value.platform_product_plan_identifier());
        let args = match (app_user_id, receipt_context) {
            (Some(app_user_id), Some(receipt_context)) => {
                PurchaseArgs::for_user_with_receipt_context(
                    product.identifier.clone(),
                    base_plan_id,
                    app_user_id,
                    receipt_context,
                )
            }
            (Some(app_user_id), None) => {
                PurchaseArgs::for_user(product.identifier.clone(), base_plan_id, app_user_id)
            }
            (None, _) => PurchaseArgs::new(product.identifier.clone(), base_plan_id),
        };
        let response: NativePurchaseResponse = self.run("purchase", args).await?;
        response.try_into_store_purchase()
    }

    async fn query_native_purchases(&self) -> Result<Vec<StorePurchase>> {
        let response: NativePurchasesResponse = self.run("queryPurchases", ()).await?;
        response
            .purchases
            .into_iter()
            .map(NativePurchaseResponse::try_into_store_purchase)
            .collect()
    }

    async fn query_native_pending_purchases(&self) -> Result<Vec<StorePurchase>> {
        let response: NativePurchasesResponse = self.run("queryPendingPurchases", ()).await?;
        response
            .purchases
            .into_iter()
            .map(NativePurchaseResponse::try_into_store_purchase)
            .collect()
    }
}

#[async_trait]
impl<R: Runtime> StoreBilling for MobileStoreBilling<R> {
    async fn is_sandbox_environment(&self) -> Result<Option<bool>> {
        let response: NativeSandboxEnvironmentResponse =
            self.run("isSandboxEnvironment", ()).await?;
        Ok(response.is_sandbox)
    }

    async fn query_products(&self, product_ids: &[String]) -> Result<Vec<StoreProduct>> {
        let response = self
            .query_native_products(QueryProductsArgs::from_product_ids(product_ids))
            .await?;
        Ok(response
            .products
            .into_iter()
            .map(|product| product.into_store_product())
            .collect())
    }

    async fn query_products_with_options(
        &self,
        queries: &[StoreProductQuery],
    ) -> Result<Vec<StoreProductResult>> {
        let response = self
            .query_native_products(QueryProductsArgs::from_queries(queries))
            .await?;
        Ok(response
            .products
            .into_iter()
            .map(|product| product.into_store_product_result())
            .collect())
    }

    async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction> {
        Ok(self
            .purchase_native(product, None, None, None)
            .await?
            .transaction)
    }

    async fn purchase_with_option(
        &self,
        product: &StoreProduct,
        option: Option<&StorePurchaseOption>,
    ) -> Result<StorePurchase> {
        self.purchase_native(product, option, None, None).await
    }

    async fn purchase_with_option_for_user(
        &self,
        product: &StoreProduct,
        option: Option<&StorePurchaseOption>,
        app_user_id: &str,
    ) -> Result<StorePurchase> {
        self.purchase_native(product, option, Some(app_user_id), None)
            .await
    }

    async fn purchase_with_receipt_context_for_user(
        &self,
        product: &StoreProduct,
        option: Option<&StorePurchaseOption>,
        app_user_id: &str,
        receipt_context: &StoreReceiptContext,
    ) -> Result<StorePurchase> {
        self.purchase_native(product, option, Some(app_user_id), Some(receipt_context))
            .await
    }

    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
        Ok(self
            .query_native_purchases()
            .await?
            .into_iter()
            .map(|purchase| purchase.transaction)
            .collect())
    }

    async fn query_purchases_with_context(&self) -> Result<Vec<StorePurchase>> {
        self.query_native_purchases().await
    }

    async fn query_pending_purchases_with_context(&self) -> Result<Vec<StorePurchase>> {
        self.query_native_pending_purchases().await
    }

    async fn finish_transaction(
        &self,
        transaction: &StoreTransaction,
        should_consume: bool,
    ) -> Result<()> {
        let _: serde_json::Value = self
            .run(
                "finishTransaction",
                FinishTransactionArgs {
                    purchase_token: transaction.purchase_token.clone(),
                    transaction_id: transaction.transaction_id.clone(),
                    should_consume,
                },
            )
            .await?;
        Ok(())
    }
}
