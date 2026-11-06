//! Typed operations over the RevenueCat HTTP API, mirroring `Backend` in the
//! official SDKs. Request/response shapes follow the wire formats of
//! purchases-android (`Backend.kt`), purchases-ios (`Operations/`), and
//! purchases-js (`backend.ts`).

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{json, Map, Value};

use crate::error::{Error, ErrorCode, Result};
use crate::http::{encode_path_segment, HttpClient, RequestOptions};
use crate::models::{
    CustomerInfo, OfferingsResponse, PlatformProductId, ProductsResponse, RedeemResult,
    TargetingResponse, VirtualCurrencies,
};

/// Body for `POST /v1/receipts`. Field names match purchases-android's
/// `Backend.postReceiptData`; the endpoint rejects unknown top-level keys, so
/// only documented fields are ever emitted and nulls are filtered out.
#[derive(Clone)]
pub struct ReceiptRequest {
    pub fetch_token: String,
    pub app_user_id: String,
    pub product_ids: Vec<String>,
    pub is_restore: bool,
    pub observer_mode: bool,
    /// `"purchase"`, `"restore"`, or `"unsynced_active_purchases"`.
    pub initiation_source: &'static str,
    /// Decimal price (`amount_micros / 1_000_000.0`).
    pub price: Option<f64>,
    pub currency: Option<String>,
    /// ISO 8601 base-plan period, e.g. `"P1M"`.
    pub normal_duration: Option<String>,
    pub presented_offering_identifier: Option<String>,
    pub presented_placement_identifier: Option<String>,
    pub applied_targeting_rule: Option<TargetingResponse>,
}

impl fmt::Debug for ReceiptRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReceiptRequest")
            .field("fetch_token", &"[REDACTED]")
            .field("app_user_id", &"[REDACTED]")
            .field("product_ids", &self.product_ids)
            .field("is_restore", &self.is_restore)
            .field("observer_mode", &self.observer_mode)
            .field("initiation_source", &self.initiation_source)
            .field("price", &self.price)
            .field("currency", &self.currency)
            .field("normal_duration", &self.normal_duration)
            .field(
                "presented_offering_identifier",
                &self.presented_offering_identifier,
            )
            .field(
                "presented_placement_identifier",
                &self.presented_placement_identifier,
            )
            .field("applied_targeting_rule", &self.applied_targeting_rule)
            .finish()
    }
}

impl ReceiptRequest {
    fn to_body_with_platform_product_ids(
        &self,
        platform_product_ids: &[PlatformProductId],
    ) -> Value {
        let mut body = Map::new();
        body.insert("fetch_token".into(), json!(self.fetch_token));
        body.insert("app_user_id".into(), json!(self.app_user_id));
        body.insert("product_ids".into(), json!(self.product_ids));
        let platform_product_ids = if platform_product_ids.is_empty() {
            self.product_ids
                .iter()
                .cloned()
                .map(PlatformProductId::new)
                .collect::<Vec<_>>()
        } else {
            platform_product_ids.to_vec()
        };
        body.insert("platform_product_ids".into(), json!(platform_product_ids));
        body.insert("is_restore".into(), json!(self.is_restore));
        body.insert("observer_mode".into(), json!(self.observer_mode));
        body.insert("purchase_completed_by".into(), json!("revenuecat"));
        body.insert("initiation_source".into(), json!(self.initiation_source));
        body.insert(
            "sdk_originated".into(),
            json!(self.initiation_source == "purchase"),
        );
        body.insert("payload_version".into(), json!(1));
        if let Some(price) = self.price {
            body.insert("price".into(), json!(price));
        }
        if let Some(currency) = &self.currency {
            body.insert("currency".into(), json!(currency));
        }
        if let Some(duration) = &self.normal_duration {
            body.insert("normal_duration".into(), json!(duration));
        }
        if let Some(offering) = &self.presented_offering_identifier {
            body.insert("presented_offering_identifier".into(), json!(offering));
        }
        if let Some(placement) = &self.presented_placement_identifier {
            body.insert("presented_placement_identifier".into(), json!(placement));
        }
        if let Some(rule) = &self.applied_targeting_rule {
            body.insert(
                "applied_targeting_rule".into(),
                json!({"rule_id": rule.rule_id, "revision": rule.revision}),
            );
        }
        Value::Object(body)
    }
}

pub(crate) struct Backend {
    http: HttpClient,
}

pub(crate) struct PostReceiptResult {
    pub customer_info: CustomerInfo,
    pub should_consume: Option<bool>,
}

fn should_consume_from_response(response: &Value, product_ids: &[String]) -> Option<bool> {
    let purchased_products = response
        .get("purchased_products")
        .and_then(Value::as_object)?;
    product_ids.iter().find_map(|product_id| {
        purchased_products
            .get(product_id)
            .and_then(|product| product.get("should_consume"))
            .and_then(Value::as_bool)
    })
}

impl Backend {
    pub fn new(http: HttpClient) -> Self {
        Self { http }
    }

    /// `GET /v1/subscribers/{id}` — fetches (and server-side creates, for new
    /// ids) the subscriber, returning server-computed CustomerInfo.
    /// Signature-verified with a nonce, like the official SDKs.
    pub async fn get_customer_info(&self, app_user_id: &str) -> Result<CustomerInfo> {
        let path = format!("/v1/subscribers/{}", encode_path_segment(app_user_id));
        let options = RequestOptions {
            endpoint_name: Some("get_customer"),
            ..RequestOptions::verified_with_nonce()
        };
        let response = self.http.get_with::<Value>(&path, options).await?;
        CustomerInfo::from_response_with_verification(response.value, response.verification)
    }

    /// `POST /v1/receipts` — records a purchase; retryable on 429 like the
    /// official SDKs.
    pub async fn post_receipt(
        &self,
        request: &ReceiptRequest,
        platform_product_ids: &[PlatformProductId],
    ) -> Result<PostReceiptResult> {
        let options = RequestOptions {
            retryable: true,
            verify: true,
            nonce: true,
            signed_fields: vec![
                ("app_user_id", request.app_user_id.clone()),
                ("fetch_token", request.fetch_token.clone()),
            ],
            endpoint_name: Some("post_receipt"),
        };
        let response = self
            .http
            .post_with::<Value>(
                "/v1/receipts",
                request.to_body_with_platform_product_ids(platform_product_ids),
                options,
            )
            .await?;
        let should_consume = should_consume_from_response(&response.value, &request.product_ids);
        let customer_info =
            CustomerInfo::from_response_with_verification(response.value, response.verification)?;
        Ok(PostReceiptResult {
            customer_info,
            should_consume,
        })
    }

    /// `GET /v1/subscribers/{id}/offerings` — signature-verified, no nonce.
    pub async fn get_offerings(&self, app_user_id: &str) -> Result<OfferingsResponse> {
        let path = format!(
            "/v1/subscribers/{}/offerings",
            encode_path_segment(app_user_id)
        );
        let options = RequestOptions {
            endpoint_name: Some("get_offerings"),
            ..RequestOptions::verified()
        };
        Ok(self.http.get_with(&path, options).await?.value)
    }

    /// `GET /rcbilling/v1/subscribers/{id}/products?id=a&id=b` — Web Billing
    /// product details; this is where Test Store products come from.
    pub async fn get_web_billing_products(
        &self,
        app_user_id: &str,
        product_ids: &[String],
    ) -> Result<ProductsResponse> {
        let query = product_ids
            .iter()
            .map(|id| format!("id={}", encode_path_segment(id)))
            .collect::<Vec<_>>()
            .join("&");
        let path = format!(
            "/rcbilling/v1/subscribers/{}/products?{query}",
            encode_path_segment(app_user_id)
        );
        self.http.get(&path).await
    }

    /// `POST /v1/subscribers/identify` — aliases `app_user_id` to
    /// `new_app_user_id`. `created` is derived from HTTP 201, not the body.
    pub async fn log_in(
        &self,
        app_user_id: &str,
        new_app_user_id: &str,
    ) -> Result<(CustomerInfo, bool)> {
        if new_app_user_id.trim().is_empty() {
            return Err(Error::new(
                ErrorCode::InvalidAppUserIdError,
                "appUserID must not be empty.",
            ));
        }
        let body = json!({"app_user_id": app_user_id, "new_app_user_id": new_app_user_id});
        let options = RequestOptions {
            verify: true,
            nonce: true,
            signed_fields: vec![
                ("app_user_id", app_user_id.to_owned()),
                ("new_app_user_id", new_app_user_id.to_owned()),
            ],
            endpoint_name: Some("log_in"),
            ..RequestOptions::default()
        };
        let response = self
            .http
            .post_with::<Value>("/v1/subscribers/identify", body, options)
            .await?;
        let created = response.status == 201;
        Ok((
            CustomerInfo::from_response_with_verification(response.value, response.verification)?,
            created,
        ))
    }

    /// `POST /v1/subscribers/{id}/attributes`. `None` values delete keys.
    pub async fn post_subscriber_attributes(
        &self,
        app_user_id: &str,
        attributes: &BTreeMap<String, Option<String>>,
    ) -> Result<()> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let body: Map<String, Value> = attributes
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    json!({"value": value, "updated_at_ms": now_ms}),
                )
            })
            .collect();
        let path = format!(
            "/v1/subscribers/{}/attributes",
            encode_path_segment(app_user_id)
        );
        let _: Value = self.http.post(&path, json!({"attributes": body})).await?;
        Ok(())
    }

    /// `GET /v1/subscribers/{id}/virtual_currencies` — verified with nonce.
    pub async fn get_virtual_currencies(&self, app_user_id: &str) -> Result<VirtualCurrencies> {
        let path = format!(
            "/v1/subscribers/{}/virtual_currencies",
            encode_path_segment(app_user_id)
        );
        let options = RequestOptions {
            endpoint_name: Some("get_virtual_currencies"),
            ..RequestOptions::verified_with_nonce()
        };
        Ok(self.http.get_with(&path, options).await?.value)
    }

    /// `POST {diagnostics_host}/v1/diagnostics` — batched diagnostics
    /// entries wrapped as `{"entries": [...]}`.
    pub async fn post_diagnostics(&self, diagnostics_url: &str, entries: &[Value]) -> Result<()> {
        let url = format!("{diagnostics_url}/v1/diagnostics");
        let _ = self
            .http
            .post_absolute(&url, json!({"entries": entries}))
            .await?;
        Ok(())
    }

    /// `POST /v1/subscribers/redeem_purchase` — attaches a web purchase to
    /// this app user. Typed outcomes follow `RedeemWebPurchaseListener.Result`:
    /// backend 7849 => InvalidToken, 7853 => Expired (with the
    /// server-obfuscated email), 7852 => PurchaseBelongsToOtherUser.
    pub async fn post_redeem_web_purchase(
        &self,
        app_user_id: &str,
        redemption_token: &str,
    ) -> Result<RedeemResult> {
        let body = json!({"redemption_token": redemption_token, "app_user_id": app_user_id});
        let options = RequestOptions {
            retryable: true,
            verify: true,
            nonce: true,
            endpoint_name: Some("post_redeem_web_purchase"),
            ..RequestOptions::default()
        };
        let response = self
            .http
            .post_with::<Value>("/v1/subscribers/redeem_purchase", body, options)
            .await;

        match response {
            Ok(response) => Ok(RedeemResult::Success {
                customer_info: CustomerInfo::from_response_with_verification(
                    response.value,
                    response.verification,
                )?,
            }),
            Err(error) => Ok(match error.backend_code {
                Some(7849) => RedeemResult::InvalidToken,
                Some(7852) => RedeemResult::PurchaseBelongsToOtherUser,
                Some(7853) => {
                    let obfuscated_email = error
                        .error_body
                        .as_ref()
                        .and_then(|body| body.get("purchase_redemption_error_info"))
                        .and_then(|info| info.get("obfuscated_email"))
                        .and_then(|email| email.as_str())
                        .map(str::to_owned);
                    match obfuscated_email {
                        Some(obfuscated_email) => RedeemResult::Expired { obfuscated_email },
                        // Missing email degrades to Error, matching Android.
                        None => RedeemResult::Error { error },
                    }
                }
                _ => RedeemResult::Error { error },
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PlatformProductId;

    #[test]
    fn receipt_request_debug_redacts_the_fetch_token_and_app_user_id() {
        let request = ReceiptRequest {
            fetch_token: "secret-store-receipt".into(),
            app_user_id: "private-user@example.com".into(),
            product_ids: vec!["monthly".into()],
            is_restore: false,
            observer_mode: false,
            initiation_source: "purchase",
            price: None,
            currency: None,
            normal_duration: None,
            presented_offering_identifier: None,
            presented_placement_identifier: None,
            applied_targeting_rule: None,
        };

        let debug = format!("{request:?}");

        assert!(!debug.contains("secret-store-receipt"));
        assert!(!debug.contains("private-user@example.com"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn receipt_body_filters_null_fields_and_flattens_offering_context() {
        // Arrange
        let request = ReceiptRequest {
            fetch_token: "test_1_x".into(),
            app_user_id: "u".into(),
            product_ids: vec!["monthly".into()],
            is_restore: false,
            observer_mode: false,
            initiation_source: "purchase",
            price: Some(3.0),
            currency: Some("USD".into()),
            normal_duration: Some("P1M".into()),
            presented_offering_identifier: Some("offering_1".into()),
            presented_placement_identifier: None,
            applied_targeting_rule: None,
        };

        // Act
        let body = request.to_body_with_platform_product_ids(&[]);

        // Assert
        assert_eq!(body["fetch_token"], "test_1_x");
        assert_eq!(body["platform_product_ids"][0]["product_id"], "monthly");
        assert_eq!(body["purchase_completed_by"], "revenuecat");
        assert_eq!(body["payload_version"], 1);
        assert_eq!(body["presented_offering_identifier"], "offering_1");
        // Null-valued optionals are absent, mirroring filterNotNullValues().
        assert!(body.get("presented_placement_identifier").is_none());
        assert!(body.get("applied_targeting_rule").is_none());
    }

    #[test]
    fn restore_receipts_flag_the_restore_initiation_source() {
        let request = ReceiptRequest {
            fetch_token: "test_1_x".into(),
            app_user_id: "u".into(),
            product_ids: vec!["monthly".into()],
            is_restore: true,
            observer_mode: false,
            initiation_source: "restore",
            price: None,
            currency: None,
            normal_duration: None,
            presented_offering_identifier: None,
            presented_placement_identifier: None,
            applied_targeting_rule: None,
        };

        let body = request.to_body_with_platform_product_ids(&[]);

        assert_eq!(body["is_restore"], true);
        assert_eq!(body["initiation_source"], "restore");
        // Optional pricing fields disappear entirely.
        assert!(body.get("price").is_none());
        assert!(body.get("currency").is_none());
        assert!(body.get("normal_duration").is_none());
    }

    #[test]
    fn targeting_rule_serializes_with_rule_id_and_revision() {
        let request = ReceiptRequest {
            fetch_token: "test_1_x".into(),
            app_user_id: "u".into(),
            product_ids: vec!["monthly".into()],
            is_restore: false,
            observer_mode: false,
            initiation_source: "purchase",
            price: None,
            currency: None,
            normal_duration: None,
            presented_offering_identifier: Some("default".into()),
            presented_placement_identifier: Some("onboarding".into()),
            applied_targeting_rule: Some(TargetingResponse {
                rule_id: "abc".into(),
                revision: 7,
            }),
        };

        let body = request.to_body_with_platform_product_ids(&[]);

        assert_eq!(body["applied_targeting_rule"]["rule_id"], "abc");
        assert_eq!(body["applied_targeting_rule"]["revision"], 7);
        assert_eq!(body["presented_placement_identifier"], "onboarding");
    }

    #[test]
    fn receipt_body_preserves_current_google_platform_product_identifiers() {
        // Arrange: shape copied from purchases-android 11.1 ReceiptInfo.
        let request = ReceiptRequest {
            fetch_token: "play-token".into(),
            app_user_id: "u".into(),
            product_ids: vec!["monthly".into()],
            is_restore: false,
            observer_mode: false,
            initiation_source: "purchase",
            price: None,
            currency: None,
            normal_duration: None,
            presented_offering_identifier: Some("default".into()),
            presented_placement_identifier: None,
            applied_targeting_rule: Some(TargetingResponse {
                rule_id: "rule-1".into(),
                revision: 12,
            }),
        };
        let platform_ids = vec![PlatformProductId::google_with_offer(
            "monthly",
            "monthly-base",
            "intro-offer",
        )];

        // Act
        let body = request.to_body_with_platform_product_ids(&platform_ids);

        // Assert
        assert_eq!(body["platform_product_ids"][0]["product_id"], "monthly");
        assert_eq!(
            body["platform_product_ids"][0]["base_plan_id"],
            "monthly-base"
        );
        assert_eq!(body["platform_product_ids"][0]["offer_id"], "intro-offer");
        assert_eq!(body["applied_targeting_rule"]["rule_id"], "rule-1");
        assert_eq!(body["sdk_originated"], true);
    }

    #[test]
    fn receipt_response_uses_only_matching_server_consume_flags() {
        let response = serde_json::json!({
            "purchased_products": {
                "coins": {"should_consume": true},
                "lifetime": {"should_consume": false}
            }
        });

        assert_eq!(
            should_consume_from_response(&response, &["coins".into()]),
            Some(true)
        );
        assert_eq!(
            should_consume_from_response(&response, &["lifetime".into()]),
            Some(false)
        );
        assert_eq!(
            should_consume_from_response(&response, &["unknown".into()]),
            None
        );
        assert_eq!(
            should_consume_from_response(&serde_json::json!({}), &["coins".into()]),
            None
        );
    }

    #[test]
    fn receipt_response_uses_the_first_matching_consume_flag() {
        let response = serde_json::json!({
            "purchased_products": {
                "coins": {"should_consume": true},
                "lifetime": {"should_consume": false}
            }
        });

        assert_eq!(
            should_consume_from_response(&response, &["lifetime".into(), "coins".into()]),
            Some(false)
        );
        assert_eq!(
            should_consume_from_response(&response, &["coins".into(), "lifetime".into()]),
            Some(true)
        );
    }
}
