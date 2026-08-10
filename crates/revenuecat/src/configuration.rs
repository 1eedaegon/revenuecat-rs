//! SDK configuration, mirroring `PurchasesConfiguration` (Android) and
//! `Configuration.Builder` (iOS).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::billing::StoreBilling;
use crate::error::{Error, ErrorCode, Result};

pub const DEFAULT_API_BASE: &str = "https://api.revenuecat.com";
pub const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// CustomerInfo/offerings cache staleness window used by the mobile SDKs in
/// foreground (`CACHE_REFRESH_PERIOD_IN_FOREGROUND`).
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(60 * 5);

/// Value sent in the `X-Platform` header. The RevenueCat backend uses this to
/// interpret `fetch_token` in `POST /v1/receipts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Ios,
    Android,
    Macos,
    Amazon,
    Stripe,
    Paddle,
    Roku,
}

impl Platform {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
            Self::Macos => "macos",
            Self::Amazon => "amazon",
            Self::Stripe => "stripe",
            Self::Paddle => "paddle",
            Self::Roku => "roku",
        }
    }

    /// Best-guess platform for the current build target.
    pub fn current() -> Self {
        if cfg!(target_os = "ios") {
            Self::Ios
        } else if cfg!(target_os = "android") {
            Self::Android
        } else {
            // Desktop targets default to macos, the only desktop value the
            // backend accepts for store receipts.
            Self::Macos
        }
    }
}

/// Trusted Entitlements response-signature verification mode, mirroring
/// `EntitlementVerificationMode` in the official SDKs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntitlementVerificationMode {
    /// No verification; responses carry `VerificationResult::NotRequested`.
    #[default]
    Disabled,
    /// Verify and report the result, but never fail a request over it.
    Informational,
    /// Verification failures surface as `SignatureVerificationError`.
    Enforced,
}

impl EntitlementVerificationMode {
    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::Disabled)
    }
}

/// Store kind encoded in the API key prefix, mirroring `APIKeyValidator`
/// (Android) and `Configuration.validate(apiKey:)` (iOS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyKind {
    /// `test_` — RevenueCat Test Store; purchases are simulated without a
    /// native store, which is what makes desktop/Tauri E2E testing possible.
    TestStore,
    /// `goog_` — Google Play.
    Google,
    /// `appl_` — Apple App Store.
    Apple,
    /// `amzn_` — Amazon Appstore.
    Amazon,
    /// `galx_` — Samsung Galaxy Store.
    Galaxy,
    /// No recognized prefix (legacy keys).
    Legacy,
}

impl ApiKeyKind {
    pub fn from_api_key(api_key: &str) -> Self {
        match api_key.split_once('_').map(|(prefix, _)| prefix) {
            Some("test") => Self::TestStore,
            Some("goog") => Self::Google,
            Some("appl") => Self::Apple,
            Some("amzn") => Self::Amazon,
            Some("galx") => Self::Galaxy,
            _ => Self::Legacy,
        }
    }
}

#[derive(Clone)]
pub struct Configuration {
    pub(crate) api_key: String,
    pub(crate) app_user_id: Option<String>,
    pub(crate) base_url: String,
    pub(crate) platform: Platform,
    pub(crate) platform_flavor: Option<String>,
    pub(crate) platform_flavor_version: Option<String>,
    pub(crate) http_timeout: Duration,
    pub(crate) cache_ttl: Duration,
    pub(crate) store_billing: Option<Arc<dyn StoreBilling>>,
    pub(crate) diagnostics_enabled: bool,
    pub(crate) diagnostics_url: String,
    pub(crate) verification_mode: EntitlementVerificationMode,
    /// Base64 Ed25519 root public key; `None` uses RevenueCat's production
    /// root key. Overridable so tests/mocks can sign with their own chain.
    pub(crate) verification_root_key: Option<String>,
}

impl Configuration {
    pub fn builder(api_key: impl Into<String>) -> ConfigurationBuilder {
        ConfigurationBuilder::new(api_key)
    }

    pub fn api_key_kind(&self) -> ApiKeyKind {
        ApiKeyKind::from_api_key(&self.api_key)
    }
}

impl fmt::Debug for Configuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Configuration")
            .field("api_key_kind", &self.api_key_kind())
            .field("app_user_id", &self.app_user_id)
            .field("base_url", &self.base_url)
            .field("platform", &self.platform)
            .field("has_store_billing", &self.store_billing.is_some())
            .finish_non_exhaustive()
    }
}

pub struct ConfigurationBuilder {
    api_key: String,
    app_user_id: Option<String>,
    proxy_url: Option<String>,
    platform: Platform,
    platform_flavor: Option<String>,
    platform_flavor_version: Option<String>,
    http_timeout: Duration,
    cache_ttl: Duration,
    store_billing: Option<Arc<dyn StoreBilling>>,
    diagnostics_enabled: bool,
    diagnostics_url: String,
    verification_mode: EntitlementVerificationMode,
    verification_root_key: Option<String>,
}

impl ConfigurationBuilder {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            app_user_id: None,
            proxy_url: None,
            platform: Platform::current(),
            platform_flavor: None,
            platform_flavor_version: None,
            http_timeout: DEFAULT_HTTP_TIMEOUT,
            cache_ttl: DEFAULT_CACHE_TTL,
            store_billing: None,
            diagnostics_enabled: false,
            diagnostics_url: crate::diagnostics::DEFAULT_DIAGNOSTICS_URL.to_owned(),
            verification_mode: EntitlementVerificationMode::default(),
            verification_root_key: None,
        }
    }

    /// Enables diagnostics tracking (off by default, like the official SDKs).
    /// Batched events post to the dedicated diagnostics host.
    pub fn diagnostics_enabled(mut self, enabled: bool) -> Self {
        self.diagnostics_enabled = enabled;
        self
    }

    /// Overrides the diagnostics host (tests/mocks only).
    pub fn diagnostics_url(mut self, url: impl Into<String>) -> Self {
        self.diagnostics_url = url.into().trim_end_matches('/').to_owned();
        self
    }

    /// `None` (the default) configures an anonymous user.
    pub fn app_user_id(mut self, app_user_id: impl Into<String>) -> Self {
        self.app_user_id = Some(app_user_id.into());
        self
    }

    /// Points the SDK at a different API host — same semantics as
    /// `Purchases.proxyURL` in the official SDKs. Used to run against
    /// `revenuecat-mock` in tests and the Tauri demo.
    pub fn proxy_url(mut self, url: impl Into<String>) -> Self {
        self.proxy_url = Some(url.into());
        self
    }

    pub fn platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self
    }

    /// Identifies a wrapper on top of this SDK (e.g. `"tauri"`), sent as
    /// `X-Platform-Flavor` like the official hybrid SDKs do.
    pub fn platform_flavor(
        mut self,
        flavor: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        self.platform_flavor = Some(flavor.into());
        self.platform_flavor_version = Some(version.into());
        self
    }

    pub fn http_timeout(mut self, timeout: Duration) -> Self {
        self.http_timeout = timeout;
        self
    }

    pub fn cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }

    /// Plugs a native store implementation (StoreKit/Play Billing bridge).
    /// Not needed for `test_` keys, which use the built-in simulated store.
    pub fn store_billing(mut self, billing: Arc<dyn StoreBilling>) -> Self {
        self.store_billing = Some(billing);
        self
    }

    /// Enables Trusted Entitlements response-signature verification,
    /// mirroring `entitlementVerificationMode` in the official SDKs.
    pub fn entitlement_verification_mode(mut self, mode: EntitlementVerificationMode) -> Self {
        self.verification_mode = mode;
        self
    }

    /// Overrides the Ed25519 root public key (base64) used for signature
    /// verification. Defaults to RevenueCat's production root key; override
    /// only when testing against a mock that signs with its own chain.
    pub fn verification_root_key(mut self, base64_key: impl Into<String>) -> Self {
        self.verification_root_key = Some(base64_key.into());
        self
    }

    pub fn build(self) -> Result<Configuration> {
        if self.api_key.trim().is_empty() {
            return Err(Error::new(
                ErrorCode::ConfigurationError,
                "API key must not be empty.",
            ));
        }
        let base_url = self
            .proxy_url
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned())
            .trim_end_matches('/')
            .to_owned();
        Ok(Configuration {
            api_key: self.api_key,
            app_user_id: self.app_user_id,
            base_url,
            platform: self.platform,
            platform_flavor: self.platform_flavor,
            platform_flavor_version: self.platform_flavor_version,
            http_timeout: self.http_timeout,
            cache_ttl: self.cache_ttl,
            store_billing: self.store_billing,
            diagnostics_enabled: self.diagnostics_enabled,
            diagnostics_url: self.diagnostics_url,
            verification_mode: self.verification_mode,
            verification_root_key: self.verification_root_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_api_key_kind_from_prefix() {
        assert_eq!(ApiKeyKind::from_api_key("test_abc"), ApiKeyKind::TestStore);
        assert_eq!(ApiKeyKind::from_api_key("goog_abc"), ApiKeyKind::Google);
        assert_eq!(ApiKeyKind::from_api_key("appl_abc"), ApiKeyKind::Apple);
        assert_eq!(ApiKeyKind::from_api_key("legacykey"), ApiKeyKind::Legacy);
    }

    #[test]
    fn rejects_empty_api_key() {
        let err = Configuration::builder("  ").build().unwrap_err();
        assert_eq!(err.code, ErrorCode::ConfigurationError);
    }

    #[test]
    fn verification_and_diagnostics_are_off_by_default() {
        let config = Configuration::builder("test_key").build().unwrap();
        assert_eq!(
            config.verification_mode,
            EntitlementVerificationMode::Disabled
        );
        assert!(!config.verification_mode.is_enabled());
        assert!(!config.diagnostics_enabled);
        assert_eq!(
            config.diagnostics_url,
            crate::diagnostics::DEFAULT_DIAGNOSTICS_URL
        );
    }

    #[test]
    fn builder_wires_verification_diagnostics_and_flavor() {
        let config = Configuration::builder("test_key")
            .entitlement_verification_mode(EntitlementVerificationMode::Enforced)
            .verification_root_key("cm9vdA==")
            .diagnostics_enabled(true)
            .diagnostics_url("http://127.0.0.1:9/")
            .platform_flavor("tauri", "2.0")
            .build()
            .unwrap();
        assert_eq!(
            config.verification_mode,
            EntitlementVerificationMode::Enforced
        );
        assert_eq!(config.verification_root_key.as_deref(), Some("cm9vdA=="));
        assert!(config.diagnostics_enabled);
        assert_eq!(
            config.diagnostics_url, "http://127.0.0.1:9",
            "trailing slash stripped"
        );
        assert_eq!(config.platform_flavor.as_deref(), Some("tauri"));
        assert_eq!(config.platform_flavor_version.as_deref(), Some("2.0"));
    }

    #[test]
    fn proxy_url_overrides_base_and_strips_trailing_slash() {
        let config = Configuration::builder("test_key")
            .proxy_url("http://127.0.0.1:9000/")
            .build()
            .unwrap();
        assert_eq!(config.base_url, "http://127.0.0.1:9000");
    }
}
