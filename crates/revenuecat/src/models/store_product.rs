//! Products. The wire types mirror the Web Billing products response
//! (`GET /rcbilling/v1/subscribers/{id}/products`), which is what the Test
//! Store uses on every platform; `StoreProduct` is the store-agnostic public
//! model matching the official SDKs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductType {
    Subscription,
    Consumable,
    NonConsumable,
    #[serde(other)]
    Unknown,
}

/// Price in micro-units, exactly as the backend sends it
/// (`{"amount_micros": 3000000, "currency": "USD"}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Price {
    pub amount_micros: i64,
    #[serde(alias = "currency_code")]
    pub currency: String,
}

impl Price {
    /// Localized-ish display string ("$3.00", "₩5,000", "3.00 EUR").
    pub fn formatted(&self) -> String {
        let (symbol, decimals) = currency_style(&self.currency);
        let amount = self.amount_micros as f64 / 1_000_000.0;
        match symbol {
            Some(symbol) if decimals == 0 => {
                format!("{symbol}{}", group_thousands(amount.round() as i64))
            }
            Some(symbol) => format!("{symbol}{amount:.decimals$}", decimals = decimals),
            None if decimals == 0 => format!(
                "{} {}",
                group_thousands(amount.round() as i64),
                self.currency
            ),
            None => format!("{amount:.decimals$} {}", self.currency, decimals = decimals),
        }
    }
}

fn currency_style(code: &str) -> (Option<&'static str>, usize) {
    match code {
        "USD" => (Some("$"), 2),
        "EUR" => (Some("€"), 2),
        "GBP" => (Some("£"), 2),
        "KRW" => (Some("₩"), 0),
        "JPY" => (Some("¥"), 0),
        _ => (None, 2),
    }
}

fn group_thousands(value: i64) -> String {
    let digits = value.abs().to_string();
    let grouped = digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(",");
    if value < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

// ---------------------------------------------------------------------------
// Wire types (Web Billing products response)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ProductsResponse {
    pub product_details: Vec<WebBillingProduct>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebBillingProduct {
    pub identifier: String,
    pub product_type: ProductType,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default_purchase_option_id: Option<String>,
    #[serde(default)]
    pub purchase_options: BTreeMap<String, PurchaseOption>,
}

/// Polymorphic: subscription options carry `base`/`trial`/`intro_price`
/// phases; one-time options carry `base_price`.
#[derive(Debug, Clone, Deserialize)]
pub struct PurchaseOption {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub price_id: Option<String>,
    #[serde(default)]
    pub base: Option<PricingPhase>,
    #[serde(default)]
    pub trial: Option<PricingPhase>,
    #[serde(default)]
    pub intro_price: Option<PricingPhase>,
    #[serde(default)]
    pub base_price: Option<Price>,
    #[serde(default)]
    pub discount: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingPhase {
    /// ISO 8601 duration, e.g. `"P1M"`.
    #[serde(default)]
    pub period_duration: Option<String>,
    #[serde(default = "default_cycle_count")]
    pub cycle_count: i64,
    #[serde(default)]
    pub price: Option<Price>,
}

fn default_cycle_count() -> i64 {
    1
}

// ---------------------------------------------------------------------------
// Public model
// ---------------------------------------------------------------------------

/// Store-agnostic product, mirroring `StoreProduct` in the official SDKs.
#[derive(Debug, Clone, Serialize)]
pub struct StoreProduct {
    pub identifier: String,
    pub product_type: ProductType,
    pub title: String,
    pub description: Option<String>,
    pub price: Price,
    /// ISO 8601 subscription period (`"P1M"`), when applicable.
    pub subscription_period: Option<String>,
    /// Free-trial phase, when the default purchase option has one.
    pub trial: Option<PricingPhase>,
    /// Introductory-price phase, when present.
    pub intro_price: Option<PricingPhase>,
}

impl StoreProduct {
    pub fn formatted_price(&self) -> String {
        self.price.formatted()
    }
}

impl TryFrom<&WebBillingProduct> for StoreProduct {
    type Error = crate::error::Error;

    fn try_from(product: &WebBillingProduct) -> crate::error::Result<Self> {
        let option = product
            .default_purchase_option_id
            .as_ref()
            .and_then(|id| product.purchase_options.get(id))
            .or_else(|| product.purchase_options.values().next())
            .ok_or_else(|| {
                crate::error::Error::new(
                    crate::error::ErrorCode::ProductNotAvailableForPurchaseError,
                    format!("Product '{}' has no purchase options.", product.identifier),
                )
            })?;

        let base_phase = option.base.as_ref();
        let price = base_phase
            .and_then(|phase| phase.price.clone())
            .or_else(|| option.base_price.clone())
            .ok_or_else(|| {
                crate::error::Error::new(
                    crate::error::ErrorCode::ProductNotAvailableForPurchaseError,
                    format!("Product '{}' has no price.", product.identifier),
                )
            })?;

        Ok(Self {
            identifier: product.identifier.clone(),
            product_type: product.product_type,
            title: product.title.clone(),
            description: product.description.clone(),
            price,
            subscription_period: base_phase.and_then(|phase| phase.period_duration.clone()),
            trial: option.trial.clone(),
            intro_price: option.intro_price.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRODUCTS_FIXTURE: &str = include_str!("../../tests/fixtures/products.json");

    #[test]
    fn parses_purchases_js_products_fixture() {
        // Arrange / Act
        let response: ProductsResponse = serde_json::from_str(PRODUCTS_FIXTURE).unwrap();

        // Assert
        assert_eq!(response.product_details.len(), 5);
        let monthly = &response.product_details[0];
        assert_eq!(monthly.identifier, "monthly");
        assert_eq!(monthly.product_type, ProductType::Subscription);
    }

    #[test]
    fn converts_subscription_product_with_trial_and_intro() {
        let response: ProductsResponse = serde_json::from_str(PRODUCTS_FIXTURE).unwrap();
        let raw = response
            .product_details
            .iter()
            .find(|p| p.identifier == "monthly_trial_intro")
            .unwrap();

        let product = StoreProduct::try_from(raw).unwrap();

        assert_eq!(product.price.amount_micros, 14_990_000);
        assert_eq!(product.subscription_period.as_deref(), Some("P1M"));
        assert_eq!(
            product.trial.as_ref().unwrap().period_duration.as_deref(),
            Some("P1W")
        );
        assert_eq!(product.intro_price.as_ref().unwrap().cycle_count, 6);
    }

    #[test]
    fn converts_consumable_product_via_base_price() {
        let response: ProductsResponse = serde_json::from_str(PRODUCTS_FIXTURE).unwrap();
        let raw = response
            .product_details
            .iter()
            .find(|p| p.identifier == "test-consumable-product")
            .unwrap();

        let product = StoreProduct::try_from(raw).unwrap();

        assert_eq!(product.product_type, ProductType::Consumable);
        assert_eq!(product.price.amount_micros, 1_000_000);
        assert!(product.subscription_period.is_none());
    }

    #[test]
    fn formats_prices_per_currency() {
        let usd = Price {
            amount_micros: 3_000_000,
            currency: "USD".into(),
        };
        let krw = Price {
            amount_micros: 5_500_000_000,
            currency: "KRW".into(),
        };
        let chf = Price {
            amount_micros: 9_990_000,
            currency: "CHF".into(),
        };
        assert_eq!(usd.formatted(), "$3.00");
        assert_eq!(krw.formatted(), "₩5,500");
        assert_eq!(chf.formatted(), "9.99 CHF");
    }
}
