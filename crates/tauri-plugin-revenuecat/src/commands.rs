//! App-facing Tauri commands. The plugin owns a configured [`Purchases`] and
//! exposes the SDK over IPC, so a Tauri frontend drives RevenueCat entirely
//! from JS/TS — no per-app Rust command glue. On mobile, real-store keys
//! (`appl_`/`goog_`) route purchases through the native store automatically.

use std::sync::Mutex;

use revenuecat::{
    ApiKeyKind, CacheFetchPolicy, Configuration, CustomerInfo, EntitlementVerificationMode, Error,
    ErrorCode, Offerings, PurchaseResult, Purchases,
};
use serde::{Deserialize, Serialize};
use tauri::{command, AppHandle, Manager, Runtime, State};

/// Managed state: the SDK instance, set by [`configure`].
#[derive(Default)]
pub struct Sdk {
    inner: Mutex<Option<Configured>>,
}

struct Configured {
    purchases: Purchases,
    store: String,
}

impl Sdk {
    /// The configured SDK, or a clear error if `configure` hasn't run yet.
    fn purchases(&self) -> Result<Purchases, Error> {
        self.lock()
            .as_ref()
            .map(|c| c.purchases.clone())
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::ConfigurationError,
                    "revenuecat is not configured; call configure() first.",
                )
            })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Configured>> {
        self.inner.lock().expect("revenuecat sdk lock poisoned")
    }
}

/// Options for [`configure`], mirroring the essentials of
/// `ConfigurationBuilder`. `platform_flavor` is set to Tauri automatically.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigureOptions {
    /// `test_` / `appl_` / `goog_` SDK key.
    pub api_key: String,
    #[serde(default)]
    pub app_user_id: Option<String>,
    /// Point at a proxy / staging / mock host (like `Purchases.proxyURL`).
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// `"disabled"` | `"informational"` | `"enforced"` (default: informational).
    #[serde(default)]
    pub entitlement_verification_mode: Option<String>,
    /// Base64 Ed25519 root key, for a custom/test signing chain.
    #[serde(default)]
    pub verification_root_key: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub configured: bool,
    pub app_user_id: Option<String>,
    pub is_anonymous: Option<bool>,
    /// `"test store"` | `"app store"` | `"play store"`.
    pub store: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    pub customer_info: CustomerInfo,
    pub created: bool,
}

fn verification_mode(name: Option<&str>) -> EntitlementVerificationMode {
    match name {
        Some("disabled") => EntitlementVerificationMode::Disabled,
        Some("enforced") => EntitlementVerificationMode::Enforced,
        // Default to Informational: verify + report, never fail a request.
        _ => EntitlementVerificationMode::Informational,
    }
}

fn info_of(configured: &Option<Configured>) -> SessionInfo {
    match configured {
        Some(c) => SessionInfo {
            configured: true,
            app_user_id: Some(c.purchases.app_user_id()),
            is_anonymous: Some(c.purchases.is_anonymous()),
            store: Some(c.store.clone()),
        },
        None => SessionInfo {
            configured: false,
            app_user_id: None,
            is_anonymous: None,
            store: None,
        },
    }
}

#[command]
pub(crate) async fn configure<R: Runtime>(
    app: AppHandle<R>,
    options: ConfigureOptions,
) -> Result<SessionInfo, Error> {
    let mut builder = Configuration::builder(options.api_key.clone())
        .platform_flavor("tauri", tauri::VERSION)
        .entitlement_verification_mode(verification_mode(
            options.entitlement_verification_mode.as_deref(),
        ));
    if let Some(user) = options.app_user_id.as_deref().filter(|u| !u.is_empty()) {
        builder = builder.app_user_id(user);
    }
    if let Some(proxy) = options.proxy_url.as_deref().filter(|p| !p.is_empty()) {
        builder = builder.proxy_url(proxy);
    }
    if let Some(root) = options.verification_root_key.as_deref() {
        builder = builder.verification_root_key(root);
    }

    // Real-store keys route purchases through the native store (mobile only);
    // a `test_` key uses the simulated Test Store with no native store.
    let store = match ApiKeyKind::from_api_key(&options.api_key) {
        ApiKeyKind::TestStore => "test store",
        _ => {
            builder = builder.store_billing(crate::store_billing(&app)?);
            if cfg!(target_os = "android") {
                "play store"
            } else {
                "app store"
            }
        }
    };

    let purchases = Purchases::configure(builder.build()?)?;
    let session = info_of(&Some(Configured {
        purchases: purchases.clone(),
        store: store.to_owned(),
    }));
    *app.state::<Sdk>().lock() = Some(Configured {
        purchases,
        store: store.to_owned(),
    });
    Ok(session)
}

#[command]
pub(crate) fn session_info(sdk: State<'_, Sdk>) -> SessionInfo {
    info_of(&sdk.lock())
}

#[command]
pub(crate) async fn get_offerings(sdk: State<'_, Sdk>) -> Result<Offerings, Error> {
    sdk.purchases()?.get_offerings().await
}

#[command]
pub(crate) async fn get_customer_info(sdk: State<'_, Sdk>) -> Result<CustomerInfo, Error> {
    sdk.purchases()?
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
}

#[command]
pub(crate) async fn purchase_package(
    sdk: State<'_, Sdk>,
    package_id: String,
) -> Result<PurchaseResult, Error> {
    let purchases = sdk.purchases()?;
    let offerings = purchases.get_offerings().await?;
    let package = offerings
        .current()
        .and_then(|offering| offering.package(&package_id))
        .ok_or_else(|| {
            Error::new(
                ErrorCode::ProductNotAvailableForPurchaseError,
                format!("Package '{package_id}' not found in the current offering."),
            )
        })?;
    purchases.purchase_package(package).await
}

#[command]
pub(crate) async fn restore(sdk: State<'_, Sdk>) -> Result<CustomerInfo, Error> {
    sdk.purchases()?.restore_purchases().await
}

#[command]
pub(crate) async fn log_in(sdk: State<'_, Sdk>, app_user_id: String) -> Result<LoginResult, Error> {
    let (customer_info, created) = sdk.purchases()?.log_in(&app_user_id).await?;
    Ok(LoginResult {
        customer_info,
        created,
    })
}

#[command]
pub(crate) async fn log_out(sdk: State<'_, Sdk>) -> Result<CustomerInfo, Error> {
    sdk.purchases()?.log_out().await
}

#[command]
pub(crate) async fn set_email(sdk: State<'_, Sdk>, email: String) -> Result<(), Error> {
    sdk.purchases()?.set_email(email).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_mode_defaults_to_informational() {
        assert_eq!(
            verification_mode(None),
            EntitlementVerificationMode::Informational
        );
        assert_eq!(
            verification_mode(Some("enforced")),
            EntitlementVerificationMode::Enforced
        );
        assert_eq!(
            verification_mode(Some("disabled")),
            EntitlementVerificationMode::Disabled
        );
    }

    #[test]
    fn unconfigured_sdk_reports_a_clear_error() {
        let sdk = Sdk::default();
        let error = sdk.purchases().unwrap_err();
        assert_eq!(error.code, ErrorCode::ConfigurationError);
        assert!(!info_of(&sdk.lock()).configured);
    }
}
