//! `StoreBilling` implementation calling the native shim over the Tauri
//! mobile plugin channel. Store calls that show UI (purchase) block until
//! the user acts, so every call runs on the blocking pool.

use async_trait::async_trait;
use revenuecat::{Error, ErrorCode, Result, StoreBilling, StoreProduct, StoreTransaction};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tauri::plugin::PluginHandle;
use tauri::Runtime;

use crate::models::{
    FinishTransactionArgs, ProductsResponse, PurchaseArgs, PurchasesResponse, QueryProductsArgs,
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
                } else {
                    Error::with_underlying(ErrorCode::StoreProblemError, message)
                }
            })
    }
}

#[async_trait]
impl<R: Runtime> StoreBilling for MobileStoreBilling<R> {
    async fn query_products(&self, product_ids: &[String]) -> Result<Vec<StoreProduct>> {
        let response: ProductsResponse = self
            .run(
                "queryProducts",
                QueryProductsArgs {
                    ids: product_ids.to_vec(),
                },
            )
            .await?;
        Ok(response.products.into_iter().map(Into::into).collect())
    }

    async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction> {
        let transaction: crate::models::PluginTransaction = self
            .run(
                "purchase",
                PurchaseArgs {
                    product_id: product.identifier.clone(),
                },
            )
            .await?;
        Ok(transaction.into())
    }

    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
        let response: PurchasesResponse = self.run("queryPurchases", ()).await?;
        Ok(response.purchases.into_iter().map(Into::into).collect())
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
