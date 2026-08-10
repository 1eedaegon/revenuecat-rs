//! Catalog and per-subscriber state for the mock backend.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::AtomicU32;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct MockProduct {
    pub identifier: String,
    /// `"subscription"`, `"consumable"`, or `"non_consumable"`.
    pub product_type: String,
    pub title: String,
    pub description: Option<String>,
    pub price_micros: i64,
    pub currency: String,
    /// ISO 8601 period for subscriptions (`"P1M"`).
    pub period: Option<String>,
    /// Entitlement granted when this product is purchased.
    pub entitlement_id: Option<String>,
}

impl MockProduct {
    pub fn subscription(
        id: &str,
        title: &str,
        price_micros: i64,
        currency: &str,
        period: &str,
        entitlement: &str,
    ) -> Self {
        Self {
            identifier: id.into(),
            product_type: "subscription".into(),
            title: title.into(),
            description: None,
            price_micros,
            currency: currency.into(),
            period: Some(period.into()),
            entitlement_id: Some(entitlement.into()),
        }
    }

    pub fn consumable(id: &str, title: &str, price_micros: i64, currency: &str) -> Self {
        Self {
            identifier: id.into(),
            product_type: "consumable".into(),
            title: title.into(),
            description: None,
            price_micros,
            currency: currency.into(),
            period: None,
            entitlement_id: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MockPackage {
    pub identifier: String,
    pub product_id: String,
}

#[derive(Debug, Clone)]
pub struct MockVirtualCurrency {
    pub code: String,
    pub name: String,
    pub balance: i64,
}

/// A configured web-purchase redemption token.
#[derive(Debug, Clone)]
pub enum MockRedemption {
    /// Redeemable once; grants the product to the redeeming user.
    Valid { product_id: String },
    /// Always answers 7853 with this obfuscated email.
    Expired { obfuscated_email: String },
}

#[derive(Debug, Clone)]
pub struct MockOffering {
    pub identifier: String,
    pub description: String,
    pub packages: Vec<MockPackage>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionEntry {
    pub purchase_date: DateTime<Utc>,
    pub original_purchase_date: DateTime<Utc>,
    pub expires_date: Option<DateTime<Utc>>,
    pub store_transaction_id: String,
}

#[derive(Debug, Clone)]
pub struct NonSubscriptionEntry {
    pub id: String,
    pub purchase_date: DateTime<Utc>,
    pub store_transaction_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct Subscriber {
    pub first_seen: Option<DateTime<Utc>>,
    pub subscriptions: BTreeMap<String, SubscriptionEntry>,
    pub non_subscriptions: BTreeMap<String, Vec<NonSubscriptionEntry>>,
    pub attributes: BTreeMap<String, Value>,
}

/// A recorded inbound request, for test assertions.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Value>,
}

#[derive(Debug)]
pub struct ServerState {
    pub products: Vec<MockProduct>,
    pub offerings: Vec<MockOffering>,
    pub current_offering_id: Option<String>,
    pub virtual_currencies: Vec<MockVirtualCurrency>,
    pub signer: crate::sign::ResponseSigner,
    pub(crate) redemptions: Mutex<HashMap<String, MockRedemption>>,
    /// redemption token -> app_user_id that redeemed it.
    pub(crate) redeemed_by: Mutex<HashMap<String, String>>,
    pub(crate) diagnostics: Mutex<Vec<serde_json::Value>>,
    /// Number of upcoming POST /v1/receipts to answer with 429.
    pub(crate) receipt_rate_limits: AtomicU32,
    /// Paths whose next ETag-capable GET answers 304 unconditionally.
    pub(crate) force_not_modified: Mutex<HashSet<String>>,
    pub(crate) subscribers: Mutex<HashMap<String, Subscriber>>,
    /// fetch_token -> app_user_id, to reject token reuse across users.
    pub(crate) used_tokens: Mutex<HashMap<String, String>>,
    pub(crate) requests: Mutex<Vec<RecordedRequest>>,
}

impl ServerState {
    pub fn new(
        products: Vec<MockProduct>,
        offerings: Vec<MockOffering>,
        current_offering_id: Option<String>,
        virtual_currencies: Vec<MockVirtualCurrency>,
        redemptions: HashMap<String, MockRedemption>,
        tamper_signatures: bool,
    ) -> Self {
        Self {
            products,
            offerings,
            current_offering_id,
            virtual_currencies,
            signer: crate::sign::ResponseSigner::new(tamper_signatures),
            redemptions: Mutex::new(redemptions),
            redeemed_by: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(Vec::new()),
            receipt_rate_limits: AtomicU32::new(0),
            force_not_modified: Mutex::new(HashSet::new()),
            subscribers: Mutex::new(HashMap::new()),
            used_tokens: Mutex::new(HashMap::new()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn product(&self, id: &str) -> Option<&MockProduct> {
        self.products.iter().find(|p| p.identifier == id)
    }

    pub fn received(&self) -> Vec<RecordedRequest> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// Diagnostics entries received on POST /v1/diagnostics.
    pub fn diagnostics(&self) -> Vec<serde_json::Value> {
        self.diagnostics
            .lock()
            .map(|d| d.clone())
            .unwrap_or_default()
    }

    pub fn subscriber(&self, app_user_id: &str) -> Option<Subscriber> {
        self.subscribers.lock().ok()?.get(app_user_id).cloned()
    }
}

/// Minimal ISO 8601 duration -> days ("P1M" -> 30). Supports Y/M/W/D parts.
pub(crate) fn iso8601_period_days(period: &str) -> Option<i64> {
    let rest = period.strip_prefix('P')?;
    let mut days = 0i64;
    let mut number = String::new();
    for c in rest.chars() {
        if c.is_ascii_digit() {
            number.push(c);
        } else {
            let n: i64 = number.parse().ok()?;
            number.clear();
            days += match c {
                'Y' => n * 365,
                'M' => n * 30,
                'W' => n * 7,
                'D' => n,
                _ => return None,
            };
        }
    }
    (number.is_empty() && days > 0).then_some(days)
}

pub(crate) fn expiry_for(period: Option<&str>, purchase: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let days = period.and_then(iso8601_period_days)?;
    Some(purchase + Duration::days(days))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_periods() {
        assert_eq!(iso8601_period_days("P1W"), Some(7));
        assert_eq!(iso8601_period_days("P1M"), Some(30));
        assert_eq!(iso8601_period_days("P3M"), Some(90));
        assert_eq!(iso8601_period_days("P1Y"), Some(365));
        assert_eq!(iso8601_period_days("bogus"), None);
    }
}
