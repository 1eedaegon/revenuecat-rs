//! Web Purchase Redemption: attaching an anonymous web purchase to the
//! current app user via a one-time deep link, mirroring
//! `WebPurchaseRedemption` / `RedeemWebPurchaseListener.Result` (Android) and
//! `WebPurchaseRedemptionResult` (iOS).

use serde::Serialize;

use super::customer_info::CustomerInfo;
use crate::error::Error;

const REDEEM_HOST: &str = "redeem_web_purchase";
const TOKEN_PARAM: &str = "redemption_token";

/// An opaque redemption token parsed from a RevenueCat deep link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebPurchaseRedemption {
    pub redemption_token: String,
}

impl WebPurchaseRedemption {
    /// Parses `<scheme>://redeem_web_purchase?redemption_token=<token>`;
    /// any scheme is accepted, host and query param names are exact.
    pub fn parse_url(url: &str) -> Option<Self> {
        let parsed = reqwest::Url::parse(url).ok()?;
        if parsed.host_str() != Some(REDEEM_HOST) {
            return None;
        }
        let token = parsed
            .query_pairs()
            .find(|(name, _)| name == TOKEN_PARAM)
            .map(|(_, value)| value.into_owned())?;
        if token.trim().is_empty() {
            return None;
        }
        Some(Self {
            redemption_token: token,
        })
    }
}

/// Mirrors the `RedeemWebPurchaseListener.Result` sealed class.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RedeemResult {
    /// Purchase attached; the returned CustomerInfo includes it.
    Success { customer_info: CustomerInfo },
    /// Backend code 7849 — the token was never valid.
    InvalidToken,
    /// Backend code 7853 — the link expired; a new one was emailed to the
    /// (server-obfuscated) address.
    Expired { obfuscated_email: String },
    /// Backend code 7852 — already redeemed by a different subscriber.
    PurchaseBelongsToOtherUser,
    /// Transport or any other backend error.
    Error { error: Error },
}

impl RedeemResult {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_redemption_links() {
        let redemption =
            WebPurchaseRedemption::parse_url("myapp://redeem_web_purchase?redemption_token=abc123")
                .unwrap();
        assert_eq!(redemption.redemption_token, "abc123");

        // Any scheme works, including https.
        assert!(
            WebPurchaseRedemption::parse_url("https://redeem_web_purchase?redemption_token=t")
                .is_some()
        );
    }

    #[test]
    fn rejects_wrong_host_missing_or_blank_token() {
        assert!(
            WebPurchaseRedemption::parse_url("myapp://other_host?redemption_token=t").is_none()
        );
        assert!(WebPurchaseRedemption::parse_url("myapp://redeem_web_purchase").is_none());
        assert!(
            WebPurchaseRedemption::parse_url("myapp://redeem_web_purchase?redemption_token=")
                .is_none()
        );
        assert!(WebPurchaseRedemption::parse_url("not a url").is_none());
    }
}
