//! Wire types shared with the native shims (camelCase JSON on the plugin
//! channel) and their conversions into the `revenuecat` crate's models.

use revenuecat::{Price, ProductType, Store, StoreProduct, StoreTransaction};
use serde::Deserialize;
#[cfg(mobile)]
use serde::Serialize;

/// The store this build's shim talks to.
pub const fn native_store() -> Store {
    if cfg!(target_os = "android") {
        Store::PlayStore
    } else if cfg!(any(target_os = "ios", target_os = "macos")) {
        Store::AppStore
    } else {
        Store::Unknown
    }
}

#[cfg(mobile)]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryProductsArgs {
    pub ids: Vec<String>,
}

#[cfg(mobile)]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseArgs {
    pub product_id: String,
}

#[cfg(mobile)]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishTransactionArgs {
    pub purchase_token: String,
    pub transaction_id: Option<String>,
    pub should_consume: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginProduct {
    pub identifier: String,
    pub product_type: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub price_micros: i64,
    pub currency: String,
    #[serde(default)]
    pub period: Option<String>,
}

#[cfg(mobile)]
#[derive(Debug, Deserialize)]
pub struct ProductsResponse {
    pub products: Vec<PluginProduct>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginTransaction {
    pub purchase_token: String,
    pub product_ids: Vec<String>,
    #[serde(default)]
    pub transaction_id: Option<String>,
    pub purchase_time_ms: i64,
}

#[cfg(mobile)]
#[derive(Debug, Deserialize)]
pub struct PurchasesResponse {
    pub purchases: Vec<PluginTransaction>,
}

impl From<PluginProduct> for StoreProduct {
    fn from(product: PluginProduct) -> Self {
        let product_type = match product.product_type.as_str() {
            "subscription" => ProductType::Subscription,
            "consumable" => ProductType::Consumable,
            "non_consumable" => ProductType::NonConsumable,
            _ => ProductType::Unknown,
        };
        StoreProduct {
            identifier: product.identifier,
            product_type,
            title: product.title,
            description: product.description,
            price: Price {
                amount_micros: product.price_micros,
                currency: product.currency,
            },
            subscription_period: product.period,
            trial: None,
            intro_price: None,
        }
    }
}

impl From<PluginTransaction> for StoreTransaction {
    fn from(transaction: PluginTransaction) -> Self {
        StoreTransaction {
            purchase_token: transaction.purchase_token,
            product_ids: transaction.product_ids,
            purchase_date: chrono::DateTime::from_timestamp_millis(transaction.purchase_time_ms)
                .unwrap_or_else(chrono::Utc::now),
            transaction_id: transaction.transaction_id,
            store: native_store(),
            price: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_plugin_product_to_store_product() {
        // Arrange: the exact JSON shape the Kotlin/Swift shims emit.
        let json = r#"{"identifier":"monthly","productType":"subscription","title":"Monthly",
            "description":null,"priceMicros":2990000,"currency":"USD","period":"P1M"}"#;

        // Act
        let plugin_product: PluginProduct = serde_json::from_str(json).unwrap();
        let product = StoreProduct::from(plugin_product);

        // Assert
        assert_eq!(product.product_type, ProductType::Subscription);
        assert_eq!(product.price.amount_micros, 2_990_000);
        assert_eq!(product.subscription_period.as_deref(), Some("P1M"));
    }

    #[test]
    fn converts_plugin_transaction_with_store_of_build_target() {
        let json = r#"{"purchaseToken":"tok","productIds":["monthly"],
            "transactionId":"GPA.123","purchaseTimeMs":1786371532643}"#;

        let transaction: PluginTransaction = serde_json::from_str(json).unwrap();
        let converted = StoreTransaction::from(transaction);

        assert_eq!(converted.purchase_token, "tok");
        assert_eq!(converted.store, native_store());
        assert_eq!(
            converted.purchase_date.timestamp_millis(),
            1_786_371_532_643
        );
    }
}
