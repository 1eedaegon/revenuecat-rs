//! `CustomerInfo` and friends, parsed from the `GET /v1/subscribers/{id}`
//! response (`{"request_date", "request_date_ms", "subscriber": {...}}`).
//! Field names match the wire fixtures shared by all three official SDKs.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorCode, Result};

/// Store that processed a purchase. String values follow the cross-platform
/// `Store` enum (12 stores) in purchases-ios `EntitlementInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Store {
    AppStore,
    MacAppStore,
    PlayStore,
    Stripe,
    Promotional,
    Amazon,
    RcBilling,
    External,
    Paddle,
    TestStore,
    Galaxy,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodType {
    Normal,
    Intro,
    Trial,
    Prepaid,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OwnershipType {
    Purchased,
    FamilyShared,
    #[serde(other)]
    Unknown,
}

/// Trusted Entitlements verification state stamped by the configured
/// response verifier. `NotRequested` is used only when verification is
/// disabled; informational/enforced modes can produce the other states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum VerificationResult {
    #[default]
    NotRequested,
    Verified,
    VerifiedOnDevice,
    Failed,
}

impl VerificationResult {
    /// Mirrors `VerificationResult.isVerified`: true for `Verified` and
    /// `VerifiedOnDevice` only.
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified | Self::VerifiedOnDevice)
    }
}

// ---------------------------------------------------------------------------
// Wire types (exact response field names)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SubscriberResponse {
    pub request_date: DateTime<Utc>,
    #[allow(dead_code)]
    pub request_date_ms: Option<i64>,
    pub subscriber: SubscriberData,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SubscriberData {
    #[serde(default)]
    pub entitlements: BTreeMap<String, EntitlementResponse>,
    pub first_seen: DateTime<Utc>,
    #[serde(default)]
    pub last_seen: Option<DateTime<Utc>>,
    #[serde(default)]
    pub management_url: Option<String>,
    #[serde(default)]
    pub non_subscriptions: BTreeMap<String, Vec<NonSubscriptionResponse>>,
    pub original_app_user_id: String,
    #[serde(default)]
    pub original_application_version: Option<String>,
    #[serde(default)]
    pub original_purchase_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub subscriptions: BTreeMap<String, SubscriptionResponse>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct EntitlementResponse {
    #[serde(default)]
    pub expires_date: Option<DateTime<Utc>>,
    pub product_identifier: String,
    #[serde(default)]
    pub product_plan_identifier: Option<String>,
    #[serde(default)]
    pub purchase_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SubscriptionResponse {
    #[serde(default)]
    pub auto_resume_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub billing_issues_detected_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub expires_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub grace_period_expires_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub is_sandbox: bool,
    #[serde(default)]
    pub original_purchase_date: Option<DateTime<Utc>>,
    #[serde(default = "default_period_type")]
    pub period_type: PeriodType,
    #[serde(default)]
    pub purchase_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub refunded_at: Option<DateTime<Utc>>,
    #[serde(default = "default_store")]
    pub store: Store,
    #[serde(default)]
    pub store_transaction_id: Option<String>,
    #[serde(default)]
    pub unsubscribe_detected_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub ownership_type: Option<OwnershipType>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct NonSubscriptionResponse {
    pub id: String,
    #[serde(default)]
    pub is_sandbox: bool,
    pub purchase_date: DateTime<Utc>,
    #[serde(default = "default_store")]
    pub store: Store,
    #[serde(default)]
    pub store_transaction_id: Option<String>,
}

fn default_period_type() -> PeriodType {
    PeriodType::Normal
}
fn default_store() -> Store {
    Store::Unknown
}

fn serialize_legacy_js_store<S>(
    store: &Store,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let legacy_store = match store {
        Store::Galaxy => Store::Unknown,
        store => *store,
    };
    legacy_store.serialize(serializer)
}

fn serialize_legacy_js_ownership<S>(
    ownership: &Option<OwnershipType>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match ownership {
        Some(OwnershipType::Unknown) | None => serializer.serialize_none(),
        Some(ownership) => serializer.serialize_some(ownership),
    }
}

// ---------------------------------------------------------------------------
// Public models
// ---------------------------------------------------------------------------

/// Mirrors `EntitlementInfo` from the official SDKs.
#[derive(Debug, Clone, Serialize)]
pub struct EntitlementInfo {
    pub identifier: String,
    pub is_active: bool,
    pub will_renew: bool,
    pub period_type: PeriodType,
    pub latest_purchase_date: Option<DateTime<Utc>>,
    pub original_purchase_date: Option<DateTime<Utc>>,
    pub expiration_date: Option<DateTime<Utc>>,
    #[serde(serialize_with = "serialize_legacy_js_store")]
    pub store: Store,
    pub product_identifier: String,
    pub product_plan_identifier: Option<String>,
    pub is_sandbox: bool,
    pub unsubscribe_detected_at: Option<DateTime<Utc>>,
    pub billing_issues_detected_at: Option<DateTime<Utc>>,
    #[serde(serialize_with = "serialize_legacy_js_ownership")]
    pub ownership_type: Option<OwnershipType>,
    pub verification: VerificationResult,
}

/// Mirrors `EntitlementInfos`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EntitlementInfos {
    pub all: BTreeMap<String, EntitlementInfo>,
    pub verification: VerificationResult,
}

impl EntitlementInfos {
    pub fn active(&self) -> BTreeMap<String, &EntitlementInfo> {
        self.all
            .iter()
            .filter(|(_, e)| e.is_active)
            .map(|(k, v)| (k.clone(), v))
            .collect()
    }

    pub fn get(&self, identifier: &str) -> Option<&EntitlementInfo> {
        self.all.get(identifier)
    }

    pub fn is_active(&self, identifier: &str) -> bool {
        self.get(identifier).map(|e| e.is_active).unwrap_or(false)
    }
}

/// One subscription entry from `subscriber.subscriptions`, keyed by product id.
#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionInfo {
    pub product_identifier: String,
    pub purchase_date: Option<DateTime<Utc>>,
    pub original_purchase_date: Option<DateTime<Utc>>,
    pub expires_date: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub will_renew: bool,
    pub period_type: PeriodType,
    #[serde(serialize_with = "serialize_legacy_js_store")]
    pub store: Store,
    pub is_sandbox: bool,
    pub unsubscribe_detected_at: Option<DateTime<Utc>>,
    pub billing_issues_detected_at: Option<DateTime<Utc>>,
    pub grace_period_expires_date: Option<DateTime<Utc>>,
    pub refunded_at: Option<DateTime<Utc>>,
    pub auto_resume_date: Option<DateTime<Utc>>,
    pub store_transaction_id: Option<String>,
    #[serde(serialize_with = "serialize_legacy_js_ownership")]
    pub ownership_type: Option<OwnershipType>,
}

/// A non-subscription (one-time) purchase.
#[derive(Debug, Clone, Serialize)]
pub struct NonSubscriptionTransaction {
    pub transaction_identifier: String,
    pub product_identifier: String,
    pub purchase_date: DateTime<Utc>,
    #[serde(serialize_with = "serialize_legacy_js_store")]
    pub store: Store,
    pub is_sandbox: bool,
    pub store_transaction_id: Option<String>,
}

/// Mirrors `CustomerInfo`: the server-computed subscription state for one
/// app user. Entitlement activeness is evaluated against the server's
/// `request_date` (not the device clock), like the official SDKs.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerInfo {
    pub request_date: DateTime<Utc>,
    pub original_app_user_id: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: Option<DateTime<Utc>>,
    pub management_url: Option<String>,
    pub original_application_version: Option<String>,
    pub original_purchase_date: Option<DateTime<Utc>>,
    pub entitlements: EntitlementInfos,
    pub subscriptions: BTreeMap<String, SubscriptionInfo>,
    pub non_subscription_transactions: Vec<NonSubscriptionTransaction>,
    /// The raw backend response, mirroring `RawDataContainer` in the SDKs.
    pub raw: serde_json::Value,
}

impl CustomerInfo {
    pub fn from_response(raw: serde_json::Value) -> Result<Self> {
        Self::from_response_with_verification(raw, VerificationResult::NotRequested)
    }

    /// Parses the subscriber envelope and stamps the given Trusted
    /// Entitlements verification result on the entitlement containers,
    /// mirroring how the official SDKs attach `VerificationResult`.
    pub fn from_response_with_verification(
        raw: serde_json::Value,
        verification: VerificationResult,
    ) -> Result<Self> {
        let response: SubscriberResponse = serde_json::from_value(raw.clone())
            .map_err(|e| Error::with_underlying(ErrorCode::CustomerInfoError, e.to_string()))?;
        let request_date = response.request_date;
        let subscriber = response.subscriber;

        let subscriptions: BTreeMap<String, SubscriptionInfo> = subscriber
            .subscriptions
            .iter()
            .map(|(product_id, sub)| {
                (
                    product_id.clone(),
                    build_subscription(product_id, sub, request_date),
                )
            })
            .collect();

        let entitlements = EntitlementInfos {
            all: subscriber
                .entitlements
                .iter()
                .map(|(id, ent)| {
                    let backing = subscriber.subscriptions.get(&ent.product_identifier);
                    let mut info = build_entitlement(id, ent, backing, request_date);
                    info.verification = verification;
                    (id.clone(), info)
                })
                .collect(),
            verification,
        };

        let non_subscription_transactions = subscriber
            .non_subscriptions
            .iter()
            .flat_map(|(product_id, transactions)| {
                transactions.iter().map(|t| NonSubscriptionTransaction {
                    transaction_identifier: t.id.clone(),
                    product_identifier: product_id.clone(),
                    purchase_date: t.purchase_date,
                    store: t.store,
                    is_sandbox: t.is_sandbox,
                    store_transaction_id: t.store_transaction_id.clone(),
                })
            })
            .collect();

        Ok(Self {
            request_date,
            original_app_user_id: subscriber.original_app_user_id,
            first_seen: subscriber.first_seen,
            last_seen: subscriber.last_seen,
            management_url: subscriber.management_url,
            original_application_version: subscriber.original_application_version,
            original_purchase_date: subscriber.original_purchase_date,
            entitlements,
            subscriptions,
            non_subscription_transactions,
            raw,
        })
    }

    /// Product identifiers with an active subscription.
    pub fn active_subscriptions(&self) -> BTreeSet<String> {
        self.subscriptions
            .iter()
            .filter(|(_, s)| s.is_active)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// All product identifiers ever purchased (subscriptions + one-time).
    pub fn all_purchased_product_ids(&self) -> BTreeSet<String> {
        self.subscriptions
            .keys()
            .cloned()
            .chain(
                self.non_subscription_transactions
                    .iter()
                    .map(|t| t.product_identifier.clone()),
            )
            .collect()
    }

    pub fn latest_expiration_date(&self) -> Option<DateTime<Utc>> {
        self.subscriptions
            .values()
            .filter_map(|s| s.expires_date)
            .max()
    }

    /// Returns the unmodified subscription object from the backend response.
    ///
    /// This is the source-compatible escape hatch for additive RevenueCat
    /// fields that are newer than this crate's typed 1.x models (for example
    /// `display_name`, `price`, `product_plan_identifier`, or
    /// `management_url`). Unknown fields remain available here because
    /// [`CustomerInfo::raw`] retains the complete response.
    pub fn raw_subscription(&self, product_identifier: &str) -> Option<&serde_json::Value> {
        self.raw
            .get("subscriber")?
            .get("subscriptions")?
            .get(product_identifier)
    }

    /// Returns the unmodified entitlement object from the backend response.
    /// This preserves future additive fields without changing the public
    /// [`EntitlementInfo`] struct layout in a 1.x release.
    pub fn raw_entitlement(&self, entitlement_identifier: &str) -> Option<&serde_json::Value> {
        self.raw
            .get("subscriber")?
            .get("entitlements")?
            .get(entitlement_identifier)
    }

    /// Returns the exact `ownership_type` wire value for a subscription.
    ///
    /// In particular, current RevenueCat responses can return `ASSIGNED` for
    /// organization-provided access. The existing exhaustive
    /// [`OwnershipType`] enum maps new values to `Unknown` to remain source
    /// compatible; this method lets callers distinguish them immediately.
    pub fn subscription_ownership_type_raw(&self, product_identifier: &str) -> Option<&str> {
        self.raw_subscription(product_identifier)?
            .get("ownership_type")?
            .as_str()
    }

    /// Returns the exact ownership wire value for the subscription backing an
    /// entitlement, including the current `ASSIGNED` value.
    pub fn entitlement_ownership_type_raw(&self, entitlement_identifier: &str) -> Option<&str> {
        let entitlement = self.raw_entitlement(entitlement_identifier)?;
        if let Some(ownership_type) = entitlement.get("ownership_type").and_then(|v| v.as_str()) {
            return Some(ownership_type);
        }
        let product_identifier = entitlement.get("product_identifier")?.as_str()?;
        self.subscription_ownership_type_raw(product_identifier)
    }
}

/// Device-clock-independent activity check, mirroring
/// `CustomerInfo+ActiveDates.swift`: compare against the server's
/// `request_date` unless it is older than a 3-day grace period, in which case
/// the local clock takes over (stale cached responses must eventually expire).
fn is_active_at(expires_date: Option<DateTime<Utc>>, request_date: DateTime<Utc>) -> bool {
    const REQUEST_DATE_GRACE: chrono::Duration = chrono::Duration::days(3);
    let Some(expires) = expires_date else {
        return true; // lifetime / non-expiring
    };
    let now = Utc::now();
    let reference = if now.signed_duration_since(request_date) <= REQUEST_DATE_GRACE {
        request_date
    } else {
        now
    };
    expires >= reference
}

fn will_renew(
    store: Store,
    expires_date: Option<DateTime<Utc>>,
    unsubscribe_detected_at: Option<DateTime<Utc>>,
    billing_issues_detected_at: Option<DateTime<Utc>>,
    period_type: PeriodType,
) -> bool {
    let is_promo = store == Store::Promotional;
    let is_lifetime = expires_date.is_none();
    let is_prepaid = period_type == PeriodType::Prepaid;
    !(is_promo
        || is_lifetime
        || is_prepaid
        || unsubscribe_detected_at.is_some()
        || billing_issues_detected_at.is_some())
}

fn build_subscription(
    product_id: &str,
    sub: &SubscriptionResponse,
    request_date: DateTime<Utc>,
) -> SubscriptionInfo {
    SubscriptionInfo {
        product_identifier: product_id.to_owned(),
        purchase_date: sub.purchase_date,
        original_purchase_date: sub.original_purchase_date,
        expires_date: sub.expires_date,
        is_active: is_active_at(sub.expires_date, request_date),
        will_renew: will_renew(
            sub.store,
            sub.expires_date,
            sub.unsubscribe_detected_at,
            sub.billing_issues_detected_at,
            sub.period_type,
        ),
        period_type: sub.period_type,
        store: sub.store,
        is_sandbox: sub.is_sandbox,
        unsubscribe_detected_at: sub.unsubscribe_detected_at,
        billing_issues_detected_at: sub.billing_issues_detected_at,
        grace_period_expires_date: sub.grace_period_expires_date,
        refunded_at: sub.refunded_at,
        auto_resume_date: sub.auto_resume_date,
        store_transaction_id: sub.store_transaction_id.clone(),
        ownership_type: sub.ownership_type,
    }
}

fn build_entitlement(
    identifier: &str,
    ent: &EntitlementResponse,
    backing_subscription: Option<&SubscriptionResponse>,
    request_date: DateTime<Utc>,
) -> EntitlementInfo {
    let store = backing_subscription
        .map(|s| s.store)
        .unwrap_or(Store::Unknown);
    let unsubscribe = backing_subscription.and_then(|s| s.unsubscribe_detected_at);
    let billing_issue = backing_subscription.and_then(|s| s.billing_issues_detected_at);
    EntitlementInfo {
        identifier: identifier.to_owned(),
        is_active: is_active_at(ent.expires_date, request_date),
        will_renew: will_renew(
            store,
            ent.expires_date,
            unsubscribe,
            billing_issue,
            backing_subscription
                .map(|s| s.period_type)
                .unwrap_or(PeriodType::Normal),
        ),
        period_type: backing_subscription
            .map(|s| s.period_type)
            .unwrap_or(PeriodType::Normal),
        latest_purchase_date: ent.purchase_date,
        original_purchase_date: backing_subscription
            .and_then(|s| s.original_purchase_date)
            .or(ent.purchase_date),
        expiration_date: ent.expires_date,
        store,
        product_identifier: ent.product_identifier.clone(),
        product_plan_identifier: ent.product_plan_identifier.clone(),
        is_sandbox: backing_subscription.map(|s| s.is_sandbox).unwrap_or(false),
        unsubscribe_detected_at: unsubscribe,
        billing_issues_detected_at: billing_issue,
        ownership_type: backing_subscription.and_then(|s| s.ownership_type),
        verification: VerificationResult::NotRequested,
    }
}

#[cfg(test)]
mod tests;
