//! SDK configuration, mirroring `PurchasesConfiguration` (Android) and
//! `Configuration.Builder` (iOS).

use std::fmt;
use std::net::IpAddr;
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
    pub(crate) sandbox_environment: Option<bool>,
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
            .field(
                "app_user_id",
                &self.app_user_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field("base_url", &"[REDACTED]")
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
    sandbox_environment: Option<bool>,
    http_timeout: Duration,
    cache_ttl: Duration,
    store_billing: Option<Arc<dyn StoreBilling>>,
    diagnostics_enabled: bool,
    diagnostics_url: String,
    verification_mode: EntitlementVerificationMode,
    verification_root_key: Option<String>,
    dangerously_allow_test_store_in_release_builds: bool,
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
            sandbox_environment: None,
            http_timeout: DEFAULT_HTTP_TIMEOUT,
            cache_ttl: DEFAULT_CACHE_TTL,
            store_billing: None,
            diagnostics_enabled: false,
            diagnostics_url: crate::diagnostics::DEFAULT_DIAGNOSTICS_URL.to_owned(),
            verification_mode: EntitlementVerificationMode::default(),
            verification_root_key: None,
            dangerously_allow_test_store_in_release_builds: false,
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

    /// Supplies the native store's current sandbox state for the
    /// `X-Is-Sandbox` request header.
    ///
    /// This is normally populated by a platform bridge. When omitted, Test
    /// Store keys still imply sandbox while production keys omit the header.
    pub fn sandbox_environment(mut self, is_sandbox: bool) -> Self {
        self.sandbox_environment = Some(is_sandbox);
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

    /// Allows a Test Store key in an iOS/Android release build.
    ///
    /// RevenueCat's official SDKs reject this by default because shipping a
    /// `test_` key can make production purchases grant test entitlements.
    /// Desktop/server release builds remain allowed without this setting so
    /// Test Store CI and Tauri desktop development keep working.
    pub fn dangerously_allow_test_store_in_release_builds(mut self, allowed: bool) -> Self {
        self.dangerously_allow_test_store_in_release_builds = allowed;
        self
    }

    pub fn build(self) -> Result<Configuration> {
        if self.api_key.trim().is_empty() {
            return Err(Error::new(
                ErrorCode::ConfigurationError,
                "API key must not be empty.",
            ));
        }
        if self.app_user_id.as_deref().is_some_and(|app_user_id| {
            !app_user_id.trim().is_empty() && !crate::identity::is_valid_app_user_id(app_user_id)
        }) {
            return Err(Error::new(
                ErrorCode::InvalidAppUserIdError,
                format!(
                    "appUserID must be at most {} UTF-8 bytes.",
                    crate::identity::MAX_APP_USER_ID_BYTES
                ),
            ));
        }
        validate_test_store_release_build(
            &self.api_key,
            self.dangerously_allow_test_store_in_release_builds,
            cfg!(debug_assertions),
            cfg!(any(target_os = "ios", target_os = "android")),
        )?;
        let base_url = match self.proxy_url {
            Some(proxy_url) => {
                validate_proxy_url(&proxy_url)?;
                proxy_url
            }
            None => DEFAULT_API_BASE.to_owned(),
        }
        .trim_end_matches('/')
        .to_owned();
        Ok(Configuration {
            api_key: self.api_key,
            app_user_id: self.app_user_id,
            base_url,
            platform: self.platform,
            platform_flavor: self.platform_flavor,
            platform_flavor_version: self.platform_flavor_version,
            sandbox_environment: self.sandbox_environment,
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

fn validate_test_store_release_build(
    api_key: &str,
    dangerously_allowed: bool,
    is_debug_build: bool,
    is_native_mobile_target: bool,
) -> Result<()> {
    if ApiKeyKind::from_api_key(api_key) == ApiKeyKind::TestStore
        && is_native_mobile_target
        && !is_debug_build
        && !dangerously_allowed
    {
        return Err(Error::new(
            ErrorCode::ConfigurationError,
            "Test Store API keys are disabled in iOS/Android release builds. Use a production API key, or explicitly call dangerously_allow_test_store_in_release_builds(true) only for controlled testing.",
        ));
    }
    Ok(())
}

fn validate_proxy_url(proxy_url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(proxy_url).map_err(|error| {
        Error::with_underlying(
            ErrorCode::ConfigurationError,
            format!("Proxy URL is invalid: {error}"),
        )
    })?;
    if parsed.scheme() == "https" {
        return Ok(());
    }
    let is_loopback_http = parsed.scheme() == "http"
        && parsed.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
    if is_loopback_http {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::ConfigurationError,
        "Proxy URL must use HTTPS. Plain HTTP is allowed only for localhost or a loopback IP during local testing.",
    ))
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
    fn rejects_configured_app_user_ids_over_the_durable_limit() {
        let error = Configuration::builder("test_key")
            .app_user_id("x".repeat(crate::identity::MAX_APP_USER_ID_BYTES + 1))
            .build()
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::InvalidAppUserIdError);
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

    #[test]
    fn proxy_url_rejects_cleartext_non_loopback_hosts() {
        let error = Configuration::builder("test_key")
            .proxy_url("http://billing.example.com")
            .build()
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::ConfigurationError);
        assert!(error.message.contains("HTTPS"));
    }

    #[test]
    fn proxy_url_allows_https_and_loopback_http() {
        for url in [
            "https://billing.example.com",
            "http://localhost:8080",
            "http://127.0.0.1:8080",
            "http://[::1]:8080",
        ] {
            assert!(
                Configuration::builder("test_key")
                    .proxy_url(url)
                    .build()
                    .is_ok(),
                "expected {url} to be accepted"
            );
        }
    }

    #[test]
    fn configuration_debug_redacts_proxy_url_secrets() {
        let config = Configuration::builder("appl_api-key-secret")
            .app_user_id("private-user@example.com")
            .proxy_url(
                "https://proxy-user:proxy-password@proxy.example/path?access_token=query-secret",
            )
            .build()
            .unwrap();

        let debug = format!("{config:?}");

        for secret in [
            "api-key-secret",
            "proxy-user",
            "proxy-password",
            "proxy.example",
            "query-secret",
            "private-user@example.com",
        ] {
            assert!(!debug.contains(secret), "Debug exposed {secret}");
        }
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn native_release_builds_reject_test_store_keys_without_explicit_opt_in() {
        let error = validate_test_store_release_build("test_key", false, false, true).unwrap_err();
        assert_eq!(error.code, ErrorCode::ConfigurationError);

        assert!(validate_test_store_release_build("test_key", true, false, true).is_ok());
    }

    #[test]
    fn desktop_release_builds_keep_supporting_test_store() {
        assert!(validate_test_store_release_build("test_key", false, false, false).is_ok());
    }
}
