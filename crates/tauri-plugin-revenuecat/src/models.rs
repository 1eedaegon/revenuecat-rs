//! Wire types shared with the native shims (camelCase JSON on the plugin
//! channel) and their conversions into the `revenuecat` crate's models.

#[cfg(any(mobile, test))]
use std::collections::BTreeSet;
use std::fmt;

#[cfg(any(mobile, test))]
use revenuecat::{
    Error, ErrorCode, PlatformProductId, StoreProductQuery, StoreProductResult, StorePurchase,
    StoreReceiptContext, TargetingResponse,
};
use revenuecat::{Price, ProductType, Store, StoreProduct, StoreTransaction};
use serde::Deserialize;
#[cfg(any(mobile, test))]
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

#[cfg(any(mobile, test))]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryProductsArgs {
    pub ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<NativeProductQuery>,
}

#[cfg(any(mobile, test))]
impl QueryProductsArgs {
    #[cfg(mobile)]
    pub(crate) fn from_product_ids(product_ids: &[String]) -> Self {
        Self {
            ids: product_ids.to_vec(),
            queries: Vec::new(),
        }
    }

    pub(crate) fn from_queries(queries: &[StoreProductQuery]) -> Self {
        let mut ids = queries
            .iter()
            .map(|query| query.product_id().to_owned())
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        Self {
            ids,
            queries: queries.iter().map(NativeProductQuery::from).collect(),
        }
    }
}

#[cfg(any(mobile, test))]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeProductQuery {
    product_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_plan_id: Option<String>,
}

#[cfg(any(mobile, test))]
impl From<&StoreProductQuery> for NativeProductQuery {
    fn from(query: &StoreProductQuery) -> Self {
        Self {
            product_id: query.product_id().to_owned(),
            base_plan_id: query.platform_product_plan_identifier().map(str::to_owned),
        }
    }
}

#[cfg(any(mobile, test))]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseArgs {
    pub product_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_plan_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_context: Option<NativeReceiptContext>,
}

#[cfg(any(mobile, test))]
impl PurchaseArgs {
    pub(crate) fn new(product_id: impl Into<String>, base_plan_id: Option<&str>) -> Self {
        Self {
            product_id: product_id.into(),
            base_plan_id: base_plan_id.map(str::to_owned),
            app_user_id: None,
            receipt_context: None,
        }
    }

    pub(crate) fn for_user(
        product_id: impl Into<String>,
        base_plan_id: Option<&str>,
        app_user_id: &str,
    ) -> Self {
        Self {
            product_id: product_id.into(),
            base_plan_id: base_plan_id.map(str::to_owned),
            app_user_id: Some(app_user_id.to_owned()),
            receipt_context: None,
        }
    }

    pub(crate) fn for_user_with_receipt_context(
        product_id: impl Into<String>,
        base_plan_id: Option<&str>,
        app_user_id: &str,
        receipt_context: &StoreReceiptContext,
    ) -> Self {
        Self {
            product_id: product_id.into(),
            base_plan_id: base_plan_id.map(str::to_owned),
            app_user_id: Some(app_user_id.to_owned()),
            receipt_context: Some(NativeReceiptContext::from(receipt_context)),
        }
    }
}

#[cfg(any(mobile, test))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeReceiptContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    presented_offering_identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    presented_placement_identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    applied_targeting_rule: Option<NativeTargetingRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    price_micros: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    normal_duration: Option<String>,
    #[serde(default)]
    sdk_originated: bool,
}

#[cfg(any(mobile, test))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeTargetingRule {
    rule_id: String,
    revision: i64,
}

#[cfg(any(mobile, test))]
impl From<&StoreReceiptContext> for NativeReceiptContext {
    fn from(context: &StoreReceiptContext) -> Self {
        Self {
            presented_offering_identifier: context
                .presented_offering_identifier()
                .map(str::to_owned),
            presented_placement_identifier: context
                .presented_placement_identifier()
                .map(str::to_owned),
            applied_targeting_rule: context.applied_targeting_rule().map(|rule| {
                NativeTargetingRule {
                    rule_id: rule.rule_id.clone(),
                    revision: rule.revision,
                }
            }),
            price_micros: context.price().map(|price| price.amount_micros),
            currency: context.price().map(|price| price.currency.clone()),
            normal_duration: context.normal_duration().map(str::to_owned),
            sdk_originated: context.sdk_originated(),
        }
    }
}

#[cfg(any(mobile, test))]
impl NativeReceiptContext {
    fn try_into_store_receipt_context(self) -> revenuecat::Result<StoreReceiptContext> {
        self.validate()?;
        let price = self
            .price_micros
            .zip(self.currency)
            .map(|(amount_micros, currency)| Price {
                amount_micros,
                currency,
            });
        Ok(StoreReceiptContext::new(
            self.presented_offering_identifier,
            self.presented_placement_identifier,
            self.applied_targeting_rule.map(|rule| TargetingResponse {
                rule_id: rule.rule_id,
                revision: rule.revision,
            }),
            price,
            self.normal_duration,
            self.sdk_originated,
        ))
    }

    fn validate(&self) -> revenuecat::Result<()> {
        let has_invalid_optional_text = [
            self.presented_offering_identifier.as_deref(),
            self.presented_placement_identifier.as_deref(),
            self.currency.as_deref(),
            self.normal_duration.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| value.trim().is_empty());
        let has_invalid_targeting = self
            .applied_targeting_rule
            .as_ref()
            .is_some_and(|rule| rule.rule_id.trim().is_empty());
        let has_partial_price = self.price_micros.is_some() != self.currency.is_some();
        let has_negative_price = self.price_micros.is_some_and(|price| price < 0);
        if has_invalid_optional_text
            || has_invalid_targeting
            || has_partial_price
            || has_negative_price
        {
            return Err(invalid_native_purchase(
                "Native receipt attribution context is invalid.",
            ));
        }
        Ok(())
    }
}

#[cfg(mobile)]
#[derive(Serialize)]
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
pub(crate) struct NativeProductsResponse {
    pub products: Vec<NativeProductResponse>,
}

#[cfg(any(mobile, test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeProductResponse {
    #[serde(flatten)]
    product: PluginProduct,
    #[serde(default)]
    base_plan_id: Option<String>,
}

#[cfg(any(mobile, test))]
impl NativeProductResponse {
    #[cfg(mobile)]
    pub(crate) fn into_store_product(self) -> StoreProduct {
        self.product.into()
    }

    pub(crate) fn into_store_product_result(self) -> StoreProductResult {
        let product = self.product.into();
        match self.base_plan_id {
            Some(base_plan_id) => StoreProductResult::for_plan(product, base_plan_id),
            None => StoreProductResult::new(product),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginTransaction {
    pub purchase_token: String,
    pub product_ids: Vec<String>,
    #[serde(default)]
    pub transaction_id: Option<String>,
    pub purchase_time_ms: i64,
}

impl fmt::Debug for PluginTransaction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginTransaction")
            .field("purchase_token", &"[REDACTED]")
            .field("product_ids", &self.product_ids)
            .field("transaction_id", &self.transaction_id)
            .field("purchase_time_ms", &self.purchase_time_ms)
            .finish()
    }
}

#[cfg(any(mobile, test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeSandboxEnvironmentResponse {
    #[serde(default)]
    pub is_sandbox: Option<bool>,
}

#[cfg(mobile)]
#[derive(Deserialize)]
pub(crate) struct NativePurchasesResponse {
    pub purchases: Vec<NativePurchaseResponse>,
}

#[cfg(any(mobile, test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativePlatformProductId {
    product_id: String,
    #[serde(default)]
    base_plan_id: Option<String>,
    #[serde(default)]
    offer_id: Option<String>,
}

#[cfg(any(mobile, test))]
impl From<NativePlatformProductId> for PlatformProductId {
    fn from(identifier: NativePlatformProductId) -> Self {
        match (identifier.base_plan_id, identifier.offer_id) {
            (Some(base_plan_id), Some(offer_id)) => {
                Self::google_with_offer(identifier.product_id, base_plan_id, offer_id)
            }
            (Some(base_plan_id), None) => Self::google(identifier.product_id, base_plan_id),
            (None, _) => Self::new(identifier.product_id),
        }
    }
}

#[cfg(any(mobile, test))]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativePurchaseResponse {
    #[serde(flatten)]
    transaction: PluginTransaction,
    #[serde(default)]
    platform_product_ids: Vec<NativePlatformProductId>,
    #[serde(default)]
    receipt_context: Option<NativeReceiptContext>,
}

#[cfg(any(mobile, test))]
impl NativePurchaseResponse {
    pub(crate) fn try_into_store_purchase(self) -> revenuecat::Result<StorePurchase> {
        self.validate()?;
        let transaction = self.transaction.into();
        let mut purchase = if self.platform_product_ids.is_empty() {
            StorePurchase::from_transaction(transaction)
        } else {
            let platform_product_ids = self
                .platform_product_ids
                .into_iter()
                .map(Into::into)
                .collect();
            StorePurchase::new(transaction, platform_product_ids)
        };
        if let Some(receipt_context) = self.receipt_context {
            purchase =
                purchase.with_receipt_context(receipt_context.try_into_store_receipt_context()?);
        }
        Ok(purchase)
    }

    fn validate(&self) -> revenuecat::Result<()> {
        if self.transaction.purchase_token.trim().is_empty() {
            return Err(invalid_native_purchase("Native purchase token is empty."));
        }
        if self.transaction.product_ids.is_empty()
            || self
                .transaction
                .product_ids
                .iter()
                .any(|product_id| product_id.trim().is_empty())
        {
            return Err(invalid_native_purchase(
                "Native purchase product IDs are empty.",
            ));
        }
        if self
            .transaction
            .transaction_id
            .as_ref()
            .is_some_and(|transaction_id| transaction_id.trim().is_empty())
        {
            return Err(invalid_native_purchase(
                "Native purchase transaction ID is empty.",
            ));
        }
        if chrono::DateTime::from_timestamp_millis(self.transaction.purchase_time_ms).is_none() {
            return Err(invalid_native_purchase(
                "Native purchase timestamp is invalid.",
            ));
        }
        self.validate_platform_product_ids()?;
        if let Some(receipt_context) = &self.receipt_context {
            receipt_context.validate()?;
        }
        Ok(())
    }

    fn validate_platform_product_ids(&self) -> revenuecat::Result<()> {
        let transaction_ids = self
            .transaction
            .product_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if transaction_ids.len() != self.transaction.product_ids.len() {
            return Err(invalid_native_purchase(
                "Native purchase product IDs are ambiguous.",
            ));
        }
        if self.platform_product_ids.is_empty() {
            return Ok(());
        }
        if self.platform_product_ids.iter().any(|identifier| {
            identifier.product_id.trim().is_empty()
                || identifier
                    .base_plan_id
                    .as_deref()
                    .is_some_and(|base_plan_id| base_plan_id.trim().is_empty())
                || identifier
                    .offer_id
                    .as_deref()
                    .is_some_and(|offer_id| offer_id.trim().is_empty())
                || (identifier.offer_id.is_some() && identifier.base_plan_id.is_none())
        }) {
            return Err(invalid_native_purchase(
                "Native platform product identity is invalid.",
            ));
        }
        let platform_ids = self
            .platform_product_ids
            .iter()
            .map(|identifier| identifier.product_id.as_str())
            .collect::<BTreeSet<_>>();
        if platform_ids.len() != self.platform_product_ids.len() || transaction_ids != platform_ids
        {
            return Err(invalid_native_purchase(
                "Native platform product IDs do not match the transaction.",
            ));
        }
        Ok(())
    }
}

#[cfg(any(mobile, test))]
fn invalid_native_purchase(message: &'static str) -> Error {
    Error::new(ErrorCode::StoreProblemError, message)
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
    fn converts_each_native_product_type_name() {
        for (name, expected) in [
            ("consumable", ProductType::Consumable),
            ("non_consumable", ProductType::NonConsumable),
            ("lifetime", ProductType::Unknown),
        ] {
            let json = format!(
                r#"{{"identifier":"coins","productType":"{name}","title":"Coins",
                "priceMicros":990000,"currency":"USD"}}"#
            );

            let plugin_product: PluginProduct = serde_json::from_str(&json).unwrap();

            assert_eq!(StoreProduct::from(plugin_product).product_type, expected);
        }
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

    #[test]
    fn plugin_transaction_debug_redacts_the_purchase_token() {
        let json = r#"{"purchaseToken":"secret-jws","productIds":["monthly"],
            "transactionId":"42","purchaseTimeMs":1786371532643}"#;
        let transaction: PluginTransaction = serde_json::from_str(json).unwrap();

        let debug = format!("{transaction:?}");

        assert!(!debug.contains("secret-jws"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn native_purchase_retains_google_plan_and_offer_context() {
        let json = r#"{"purchaseToken":"tok","productIds":["monthly"],
            "transactionId":"GPA.123","purchaseTimeMs":1786371532643,
            "platformProductIds":[{"productId":"monthly",
                "basePlanId":"monthly-base","offerId":"trial"}]}"#;

        let response: NativePurchaseResponse = serde_json::from_str(json).unwrap();
        let purchase = response.try_into_store_purchase().unwrap();

        assert_eq!(purchase.transaction.purchase_token, "tok");
        assert_eq!(purchase.platform_product_ids.len(), 1);
        assert_eq!(
            purchase.platform_product_ids[0].base_plan_id(),
            Some("monthly-base")
        );
        assert_eq!(purchase.platform_product_ids[0].offer_id(), Some("trial"));
    }

    #[test]
    fn native_purchase_without_context_uses_legacy_product_ids() {
        let json = r#"{"purchaseToken":"jws","productIds":["monthly"],
            "transactionId":"42","purchaseTimeMs":1786371532643}"#;

        let response: NativePurchaseResponse = serde_json::from_str(json).unwrap();
        let purchase = response.try_into_store_purchase().unwrap();

        assert_eq!(purchase.platform_product_ids.len(), 1);
        assert_eq!(purchase.platform_product_ids[0].product_id(), "monthly");
        assert_eq!(purchase.platform_product_ids[0].base_plan_id(), None);
    }

    #[test]
    fn native_product_retains_google_base_plan_context() {
        let json = r#"{"identifier":"premium","productType":"subscription",
            "title":"Premium","description":null,"priceMicros":4990000,
            "currency":"USD","period":"P1M","basePlanId":"monthly-base",
            "offerId":"trial","offerToken":"token"}"#;

        let response: NativeProductResponse = serde_json::from_str(json).unwrap();
        let result = response.into_store_product_result();

        assert_eq!(result.product().identifier, "premium");
        assert_eq!(
            result.platform_product_plan_identifier(),
            Some("monthly-base")
        );
    }

    #[test]
    fn plan_aware_product_query_serializes_legacy_ids_and_native_queries() {
        let queries = vec![
            StoreProductQuery::for_plan("premium", "monthly-base"),
            StoreProductQuery::for_plan("premium", "annual-base"),
        ];

        let serialized = serde_json::to_value(QueryProductsArgs::from_queries(&queries)).unwrap();

        assert_eq!(serialized["ids"], serde_json::json!(["premium"]));
        assert_eq!(
            serialized["queries"],
            serde_json::json!([
                {"productId": "premium", "basePlanId": "monthly-base"},
                {"productId": "premium", "basePlanId": "annual-base"}
            ])
        );
    }

    #[test]
    fn plan_aware_purchase_serializes_android_base_plan() {
        let serialized =
            serde_json::to_value(PurchaseArgs::new("premium", Some("monthly-base"))).unwrap();

        assert_eq!(
            serialized,
            serde_json::json!({
                "productId": "premium",
                "basePlanId": "monthly-base"
            })
        );
    }

    #[test]
    fn user_aware_purchase_serializes_the_current_app_user_id() {
        let serialized = serde_json::to_value(PurchaseArgs::for_user(
            "premium",
            Some("monthly-base"),
            "123e4567-e89b-12d3-a456-426614174000",
        ))
        .unwrap();

        assert_eq!(
            serialized,
            serde_json::json!({
                "productId": "premium",
                "basePlanId": "monthly-base",
                "appUserId": "123e4567-e89b-12d3-a456-426614174000"
            })
        );
    }

    #[test]
    fn purchase_serializes_receipt_context_for_native_persistence() {
        let context = StoreReceiptContext::new(
            Some("default".into()),
            Some("onboarding".into()),
            Some(TargetingResponse {
                rule_id: "rule".into(),
                revision: 7,
            }),
            Some(Price {
                amount_micros: 4_990_000,
                currency: "USD".into(),
            }),
            Some("P1M".into()),
            true,
        );

        let serialized = serde_json::to_value(PurchaseArgs::for_user_with_receipt_context(
            "premium", None, "user", &context,
        ))
        .unwrap();

        assert_eq!(
            serialized["receiptContext"],
            serde_json::json!({
                "presentedOfferingIdentifier": "default",
                "presentedPlacementIdentifier": "onboarding",
                "appliedTargetingRule": {"ruleId": "rule", "revision": 7},
                "priceMicros": 4_990_000,
                "currency": "USD",
                "normalDuration": "P1M",
                "sdkOriginated": true
            })
        );
    }

    #[test]
    fn native_purchase_rehydrates_persisted_receipt_context() {
        let json = r#"{"purchaseToken":"jws","productIds":["monthly"],
            "transactionId":"42","purchaseTimeMs":1786371532643,
            "receiptContext":{"presentedOfferingIdentifier":"default",
                "presentedPlacementIdentifier":"onboarding",
                "appliedTargetingRule":{"ruleId":"rule","revision":7},
                "priceMicros":4990000,"currency":"USD",
                "normalDuration":"P1M","sdkOriginated":true}}"#;

        let response: NativePurchaseResponse = serde_json::from_str(json).unwrap();
        let purchase = response.try_into_store_purchase().unwrap();
        let context = purchase.receipt_context().unwrap();

        assert_eq!(context.presented_offering_identifier(), Some("default"));
        assert_eq!(context.presented_placement_identifier(), Some("onboarding"));
        assert_eq!(context.applied_targeting_rule().unwrap().rule_id, "rule");
        assert_eq!(context.price().unwrap().amount_micros, 4_990_000);
        assert_eq!(context.normal_duration(), Some("P1M"));
        assert!(context.sdk_originated());
    }

    #[test]
    fn native_purchase_rejects_invalid_receipt_price_context() {
        let partial_price = r#"{"purchaseToken":"jws","productIds":["monthly"],
            "transactionId":"42","purchaseTimeMs":1786371532643,
            "receiptContext":{"priceMicros":4990000,"sdkOriginated":true}}"#;
        let negative_price = r#"{"purchaseToken":"jws","productIds":["monthly"],
            "transactionId":"42","purchaseTimeMs":1786371532643,
            "receiptContext":{"priceMicros":-1,"currency":"USD",
                "sdkOriginated":true}}"#;

        for json in [partial_price, negative_price] {
            let response: NativePurchaseResponse = serde_json::from_str(json).unwrap();
            let error = response.try_into_store_purchase().unwrap_err();

            assert_eq!(error.code, revenuecat::ErrorCode::StoreProblemError);
        }
    }

    #[test]
    fn native_purchase_accepts_a_free_receipt_price() {
        let json = r#"{"purchaseToken":"jws","productIds":["monthly"],
            "transactionId":"42","purchaseTimeMs":1786371532643,
            "receiptContext":{"priceMicros":0,"currency":"USD",
                "sdkOriginated":true}}"#;

        let response: NativePurchaseResponse = serde_json::from_str(json).unwrap();
        let purchase = response.try_into_store_purchase().unwrap();

        let context = purchase.receipt_context().unwrap();
        assert_eq!(context.price().unwrap().amount_micros, 0);
    }

    #[test]
    fn native_purchase_rejects_each_blank_receipt_attribution_field_alone() {
        let blank_offering = r#"{"presentedOfferingIdentifier":" ","sdkOriginated":true}"#;
        let blank_targeting_rule =
            r#"{"appliedTargetingRule":{"ruleId":"\t","revision":7},"sdkOriginated":true}"#;

        for receipt_context in [blank_offering, blank_targeting_rule] {
            let json = format!(
                r#"{{"purchaseToken":"jws","productIds":["monthly"],
                "transactionId":"42","purchaseTimeMs":1786371532643,
                "receiptContext":{receipt_context}}}"#
            );
            let response: NativePurchaseResponse = serde_json::from_str(&json).unwrap();
            let error = response.try_into_store_purchase().unwrap_err();

            assert_eq!(error.code, revenuecat::ErrorCode::StoreProblemError);
        }
    }

    #[test]
    fn native_purchase_rejects_missing_or_blank_product_ids() {
        for product_ids in ["[]", r#"[" "]"#] {
            let json = format!(
                r#"{{"purchaseToken":"jws","productIds":{product_ids},
                "transactionId":"42","purchaseTimeMs":1786371532643}}"#
            );
            let response: NativePurchaseResponse = serde_json::from_str(&json).unwrap();
            let error = response.try_into_store_purchase().unwrap_err();

            assert_eq!(error.code, revenuecat::ErrorCode::StoreProblemError);
        }
    }

    #[test]
    fn native_purchase_rejects_blank_plan_and_offer_identifiers() {
        let blank_plan = r#"{"purchaseToken":"tok","productIds":["monthly"],
            "transactionId":"GPA.123","purchaseTimeMs":1786371532643,
            "platformProductIds":[{"productId":"monthly","basePlanId":" "}]}"#;
        let blank_offer = r#"{"purchaseToken":"tok","productIds":["monthly"],
            "transactionId":"GPA.123","purchaseTimeMs":1786371532643,
            "platformProductIds":[{"productId":"monthly",
                "basePlanId":"monthly-base","offerId":"\t"}]}"#;

        for json in [blank_plan, blank_offer] {
            let response: NativePurchaseResponse = serde_json::from_str(json).unwrap();
            let error = response.try_into_store_purchase().unwrap_err();

            assert_eq!(error.code, revenuecat::ErrorCode::StoreProblemError);
        }
    }

    #[test]
    fn native_purchase_rejects_duplicate_or_ambiguous_product_identities() {
        let mismatched_product = r#"{"purchaseToken":"tok","productIds":["monthly"],"transactionId":"GPA.123","purchaseTimeMs":1786371532643,"platformProductIds":[{"productId":"annual","basePlanId":"annual-base"}]}"#;
        let duplicate_transaction_product = r#"{"purchaseToken":"tok","productIds":["monthly","monthly"],"transactionId":"GPA.124","purchaseTimeMs":1786371532643}"#;
        let duplicate_platform_product = r#"{"purchaseToken":"tok","productIds":["monthly"],"transactionId":"GPA.125","purchaseTimeMs":1786371532643,"platformProductIds":[{"productId":"monthly","basePlanId":"monthly-base","offerId":"trial"},{"productId":"monthly","basePlanId":"annual-base","offerId":"promo"}]}"#;
        for json in [
            mismatched_product,
            duplicate_transaction_product,
            duplicate_platform_product,
        ] {
            let response: NativePurchaseResponse = serde_json::from_str(json).unwrap();
            let error = response.try_into_store_purchase().unwrap_err();

            assert_eq!(error.code, revenuecat::ErrorCode::StoreProblemError);
        }
    }

    #[test]
    fn native_purchase_rejects_empty_tokens_and_invalid_timestamps() {
        let empty_token = r#"{"purchaseToken":" ","productIds":["monthly"],
            "transactionId":"42","purchaseTimeMs":1786371532643}"#;
        let invalid_timestamp = r#"{"purchaseToken":"jws","productIds":["monthly"],
            "transactionId":"42","purchaseTimeMs":9223372036854775807}"#;

        for json in [empty_token, invalid_timestamp] {
            let response: NativePurchaseResponse = serde_json::from_str(json).unwrap();
            let error = response.try_into_store_purchase().unwrap_err();
            assert_eq!(error.code, revenuecat::ErrorCode::StoreProblemError);
        }
    }

    #[test]
    fn native_sandbox_response_preserves_known_and_unknown_states() {
        for (json, expected) in [
            (r#"{"isSandbox":true}"#, Some(true)),
            (r#"{"isSandbox":false}"#, Some(false)),
            (r#"{"isSandbox":null}"#, None),
            (r#"{}"#, None),
        ] {
            let response: NativeSandboxEnvironmentResponse = serde_json::from_str(json).unwrap();
            assert_eq!(response.is_sandbox, expected);
        }
    }
}
