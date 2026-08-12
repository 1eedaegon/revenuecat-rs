//! Paywall configuration, delivered on each offering in the
//! `GET /v1/subscribers/{id}/offerings` response. RevenueCat has two paywall
//! generations:
//!
//! - **v1 (templates)** — the `paywall` field, a [`Paywall`] with a
//!   `template_name`, a `config` (colors, images, copy, features, packages),
//!   and per-locale `localized_strings`. Mirrors `PaywallData` in the
//!   official SDKs.
//! - **v2 (components)** — the `paywall_components` field, a declarative
//!   component tree kept here as [`PaywallComponents`] (structured wrapper +
//!   raw config) since its shape is large and evolving.
//!
//! In a Tauri app the webview renders these as HTML; the SDK only exposes the
//! typed config so the frontend can draw it (see the demo's paywall renderer).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Paywalls v1 — PaywallData
// ---------------------------------------------------------------------------

/// A dashboard-configured paywall (v1 templates). Copy is resolved per locale
/// from [`Paywall::localized_strings`], falling back to the config defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paywall {
    /// Template layout id, e.g. `"1"`..`"5"`, `"7"`.
    pub template_name: String,
    #[serde(default)]
    pub revision: i64,
    #[serde(default)]
    pub default_locale: Option<String>,
    /// Base URL for relative image assets in `config.images`.
    #[serde(default)]
    pub asset_base_url: Option<String>,
    pub config: PaywallConfig,
    /// Per-locale string overrides, keyed by locale (e.g. `"en_US"`).
    #[serde(default)]
    pub localized_strings: BTreeMap<String, PaywallLocalizedStrings>,
}

impl Paywall {
    /// Copy for `locale` (or the default locale, or the first available),
    /// merged onto the config defaults — what a renderer should display.
    pub fn strings_for(&self, locale: &str) -> PaywallLocalizedStrings {
        let localized = self
            .localized_strings
            .get(locale)
            .or_else(|| {
                self.default_locale
                    .as_ref()
                    .and_then(|d| self.localized_strings.get(d))
            })
            .or_else(|| self.localized_strings.values().next());

        PaywallLocalizedStrings {
            title: localized
                .and_then(|l| l.title.clone())
                .or_else(|| self.config.title.clone()),
            subtitle: localized
                .and_then(|l| l.subtitle.clone())
                .or_else(|| self.config.subtitle.clone()),
            call_to_action: localized
                .and_then(|l| l.call_to_action.clone())
                .or_else(|| self.config.call_to_action.clone()),
            call_to_action_with_intro_offer: localized
                .and_then(|l| l.call_to_action_with_intro_offer.clone())
                .or_else(|| self.config.call_to_action_with_intro_offer.clone()),
            offer_details: localized
                .and_then(|l| l.offer_details.clone())
                .or_else(|| self.config.offer_details.clone()),
            offer_details_with_intro_offer: localized
                .and_then(|l| l.offer_details_with_intro_offer.clone())
                .or_else(|| self.config.offer_details_with_intro_offer.clone()),
            features: localized
                .map(|l| l.features.clone())
                .filter(|f| !f.is_empty())
                .unwrap_or_else(|| self.config.features.clone()),
        }
    }

    /// Absolute URL for a `config.images` value, honoring `asset_base_url`.
    pub fn image_url(&self, image: &str) -> String {
        match &self.asset_base_url {
            Some(base) if !image.starts_with("http") => {
                format!(
                    "{}/{}",
                    base.trim_end_matches('/'),
                    image.trim_start_matches('/')
                )
            }
            _ => image.to_owned(),
        }
    }

    /// Images best suited for a webview: WebP (browser-friendly) if present,
    /// else the legacy JPG set. (HEIC is skipped — browsers can't render it.)
    pub fn web_images(&self) -> &PaywallImages {
        if self.config.images_webp.header.is_some() || self.config.images_webp.background.is_some()
        {
            &self.config.images_webp
        } else {
            &self.config.images
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaywallConfig {
    /// Package identifiers the paywall offers, in display order.
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub default_package: Option<String>,
    /// Legacy JPG images (widest browser support).
    #[serde(default)]
    pub images: PaywallImages,
    /// WebP images — preferred for a webview when present.
    #[serde(default)]
    pub images_webp: PaywallImages,
    #[serde(default)]
    pub colors: PaywallColorsByMode,
    #[serde(default)]
    pub blurb: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub call_to_action: Option<String>,
    #[serde(default)]
    pub call_to_action_with_intro_offer: Option<String>,
    #[serde(default)]
    pub offer_details: Option<String>,
    #[serde(default)]
    pub offer_details_with_intro_offer: Option<String>,
    #[serde(default)]
    pub features: Vec<PaywallFeature>,
    #[serde(default)]
    pub tos_url: Option<String>,
    #[serde(default)]
    pub privacy_url: Option<String>,
    #[serde(default)]
    pub display_restore_purchases: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PaywallImages {
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

/// Light/dark color sets. Colors are hex strings (`#RRGGBB` / `#RRGGBBAA`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PaywallColorsByMode {
    #[serde(default)]
    pub light: PaywallColors,
    #[serde(default)]
    pub dark: Option<PaywallColors>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PaywallColors {
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub text_1: Option<String>,
    #[serde(default)]
    pub text_2: Option<String>,
    #[serde(default)]
    pub text_3: Option<String>,
    #[serde(default)]
    pub call_to_action_background: Option<String>,
    #[serde(default)]
    pub call_to_action_foreground: Option<String>,
    #[serde(default)]
    pub call_to_action_secondary_background: Option<String>,
    #[serde(default)]
    pub close_button: Option<String>,
    #[serde(default)]
    pub accent_1: Option<String>,
    #[serde(default)]
    pub accent_2: Option<String>,
    #[serde(default)]
    pub accent_3: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaywallFeature {
    pub title: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub icon_id: Option<String>,
}

/// The subset of copy that can be localized. `strings_for` returns this with
/// config defaults filled in.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PaywallLocalizedStrings {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub call_to_action: Option<String>,
    #[serde(default)]
    pub call_to_action_with_intro_offer: Option<String>,
    #[serde(default)]
    pub offer_details: Option<String>,
    #[serde(default)]
    pub offer_details_with_intro_offer: Option<String>,
    #[serde(default)]
    pub features: Vec<PaywallFeature>,
}

// ---------------------------------------------------------------------------
// Paywalls v2 — Components (structured wrapper; tree kept as raw JSON)
// ---------------------------------------------------------------------------

/// A v2 component-based paywall. The component tree is large and evolving, so
/// it is exposed as raw JSON for the frontend to interpret; the envelope
/// fields a renderer needs are typed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaywallComponents {
    #[serde(default)]
    pub template_name: Option<String>,
    #[serde(default)]
    pub revision: i64,
    #[serde(default)]
    pub default_locale: Option<String>,
    #[serde(default)]
    pub asset_base_url: Option<String>,
    /// The root component tree (screens -> stacks -> components).
    #[serde(default)]
    pub components_config: serde_json::Value,
    /// Per-locale asset/string map referenced by component ids.
    #[serde(default)]
    pub components_localizations: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYWALL_JSON: &str = r##"{
        "template_name": "2",
        "revision": 7,
        "default_locale": "en_US",
        "asset_base_url": "https://assets.pawwalls.com",
        "config": {
            "packages": ["$rc_monthly", "$rc_annual"],
            "default_package": "$rc_annual",
            "images": { "header": "header.heic", "icon": "icon.png" },
            "colors": {
                "light": {
                    "background": "#ffffff",
                    "text_1": "#000000",
                    "call_to_action_background": "#f2545b",
                    "call_to_action_foreground": "#ffffff"
                }
            },
            "title": "Default Title",
            "call_to_action": "Continue",
            "features": [{ "title": "Config feature", "icon_id": "star" }],
            "display_restore_purchases": true
        },
        "localized_strings": {
            "en_US": {
                "title": "Unlock Pro",
                "subtitle": "Everything, unlocked",
                "call_to_action": "Subscribe",
                "offer_details": "{{ price }} / {{ period }}",
                "features": [
                    { "title": "Unlimited access", "icon_id": "lock" },
                    { "title": "Priority support", "icon_id": "chat" }
                ]
            }
        }
    }"##;

    #[test]
    fn parses_paywall_v1() {
        let paywall: Paywall = serde_json::from_str(PAYWALL_JSON).unwrap();
        assert_eq!(paywall.template_name, "2");
        assert_eq!(paywall.revision, 7);
        assert_eq!(paywall.config.packages, vec!["$rc_monthly", "$rc_annual"]);
        assert_eq!(
            paywall.config.default_package.as_deref(),
            Some("$rc_annual")
        );
        assert!(paywall.config.display_restore_purchases);
        assert_eq!(
            paywall
                .config
                .colors
                .light
                .call_to_action_background
                .as_deref(),
            Some("#f2545b")
        );
    }

    #[test]
    fn localized_strings_override_config_defaults() {
        let paywall: Paywall = serde_json::from_str(PAYWALL_JSON).unwrap();

        // Locale copy wins over config.
        let en = paywall.strings_for("en_US");
        assert_eq!(en.title.as_deref(), Some("Unlock Pro"));
        assert_eq!(en.call_to_action.as_deref(), Some("Subscribe"));
        assert_eq!(en.features.len(), 2);
        assert_eq!(en.features[0].title, "Unlimited access");

        // Unknown locale falls back to the default locale's copy.
        let other = paywall.strings_for("fr_FR");
        assert_eq!(other.title.as_deref(), Some("Unlock Pro"));
    }

    #[test]
    fn image_url_honors_asset_base() {
        let paywall: Paywall = serde_json::from_str(PAYWALL_JSON).unwrap();
        assert_eq!(
            paywall.image_url("header.heic"),
            "https://assets.pawwalls.com/header.heic"
        );
        // Absolute URLs pass through untouched.
        assert_eq!(paywall.image_url("https://x/y.png"), "https://x/y.png");
    }
}
