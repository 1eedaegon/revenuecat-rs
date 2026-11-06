//! App-facing Tauri commands. The plugin owns a configured [`Purchases`] and
//! exposes the SDK over IPC, so a Tauri frontend drives RevenueCat entirely
//! from JS/TS — no per-app Rust command glue. On mobile, real-store keys
//! (`appl_`/`goog_`) route purchases through the native store automatically.

use std::fmt;
use std::path::PathBuf;
use std::sync::Mutex;

use revenuecat::{
    ApiKeyKind, CacheFetchPolicy, Configuration, ConfigurationBuilder, CustomerInfo,
    EntitlementVerificationMode, Error, ErrorCode, Offerings, PurchaseResult, Purchases,
};
use serde::{Deserialize, Serialize};
use tauri::{command, AppHandle, Emitter, Manager, Runtime, State};

mod identity_persistence;

use identity_persistence::{IdentityOperation, PersistedIdentity};

/// Emitted to the webview whenever the customer info changes (purchase,
/// restore, login, …), mirroring RevenueCat's `updatedCustomerInfoListener`.
pub const CUSTOMER_INFO_UPDATED_EVENT: &str = "revenuecat:customer-info-updated";

/// Managed state: the SDK instance, set by [`configure`].
#[derive(Default)]
pub struct Sdk {
    inner: Mutex<Option<Configured>>,
    configuration_operation: tokio::sync::Mutex<()>,
}

struct Configured {
    purchases: Purchases,
    store: String,
    fingerprint: ConfigureFingerprint,
}

#[derive(Clone, PartialEq, Eq)]
struct ConfigureFingerprint {
    api_key: String,
    app_user_id: Option<String>,
    proxy_url: Option<String>,
    entitlement_verification_mode: EntitlementVerificationMode,
    verification_root_key: Option<String>,
    dangerously_allow_test_store_in_release_builds: bool,
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
#[derive(Deserialize)]
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

impl fmt::Debug for ConfigureOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfigureOptions")
            .field("api_key", &"[REDACTED]")
            .field(
                "app_user_id",
                &self.app_user_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field("proxy_url", &self.proxy_url.as_ref().map(|_| "[REDACTED]"))
            .field(
                "entitlement_verification_mode",
                &self.entitlement_verification_mode,
            )
            .field(
                "verification_root_key",
                &self.verification_root_key.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// IPC-only extension kept separate from the public 1.x `ConfigureOptions`
/// struct so existing Rust struct literals remain source compatible.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigureCommandOptions {
    #[serde(flatten)]
    base: ConfigureOptions,
    /// Allows a Test Store key in a mobile release build. This is dangerous
    /// because test purchases can grant test entitlements in production.
    #[serde(default)]
    dangerously_allow_test_store_in_release_builds: bool,
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

fn verification_mode(name: Option<&str>) -> Result<EntitlementVerificationMode, Error> {
    match name {
        Some("disabled") => Ok(EntitlementVerificationMode::Disabled),
        // Default to Informational: verify + report, never fail a request.
        Some("informational") | None => Ok(EntitlementVerificationMode::Informational),
        Some("enforced") => Ok(EntitlementVerificationMode::Enforced),
        Some(_) => Err(Error::new(
            ErrorCode::ConfigurationError,
            "entitlementVerificationMode must be 'disabled', 'informational', or 'enforced'.",
        )),
    }
}

impl ConfigureFingerprint {
    fn from_options(
        options: &ConfigureOptions,
        allows_test_store_in_release: bool,
    ) -> Result<Self, Error> {
        Ok(Self {
            api_key: options.api_key.clone(),
            app_user_id: options
                .app_user_id
                .as_deref()
                .filter(|app_user_id| !app_user_id.is_empty())
                .map(str::to_owned),
            proxy_url: options
                .proxy_url
                .as_deref()
                .filter(|proxy_url| !proxy_url.is_empty())
                .map(str::to_owned),
            entitlement_verification_mode: verification_mode(
                options.entitlement_verification_mode.as_deref(),
            )?,
            verification_root_key: options.verification_root_key.clone(),
            dangerously_allow_test_store_in_release_builds: allows_test_store_in_release,
        })
    }
}

fn info_of(configured: &Option<Configured>) -> SessionInfo {
    match configured {
        Some(configured) => info_of_configured(configured),
        None => SessionInfo {
            configured: false,
            app_user_id: None,
            is_anonymous: None,
            store: None,
        },
    }
}

fn info_of_configured(configured: &Configured) -> SessionInfo {
    SessionInfo {
        configured: true,
        app_user_id: Some(configured.purchases.app_user_id()),
        is_anonymous: Some(configured.purchases.is_anonymous()),
        store: Some(configured.store.clone()),
    }
}

fn persisted_identity_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, Error> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join(identity_persistence::identity_file_name()))
        .map_err(|error| {
            Error::with_underlying(
                ErrorCode::ConfigurationError,
                format!("Could not resolve the app data directory: {error}"),
            )
        })
}

fn existing_configuration(
    sdk: &Sdk,
    fingerprint: &ConfigureFingerprint,
) -> Result<Option<SessionInfo>, Error> {
    let configured = sdk.lock();
    match configured.as_ref() {
        Some(current) if &current.fingerprint == fingerprint => Ok(Some(info_of(&configured))),
        Some(_) => Err(Error::new(
            ErrorCode::ConfigurationError,
            "RevenueCat is already configured with different options. Configure it once per app process, then use logIn/logOut to change users.",
        )),
        None => Ok(None),
    }
}

fn base_configuration(
    options: &ConfigureOptions,
    app_user_id: &str,
    allows_test_store_in_release: bool,
) -> Result<ConfigurationBuilder, Error> {
    let mut builder = Configuration::builder(options.api_key.clone())
        .app_user_id(app_user_id)
        .platform_flavor("tauri", tauri::VERSION)
        .entitlement_verification_mode(verification_mode(
            options.entitlement_verification_mode.as_deref(),
        )?)
        .dangerously_allow_test_store_in_release_builds(allows_test_store_in_release);
    if let Some(proxy) = options
        .proxy_url
        .as_deref()
        .filter(|proxy| !proxy.is_empty())
    {
        builder = builder.proxy_url(proxy);
    }
    if let Some(root) = options.verification_root_key.as_deref() {
        builder = builder.verification_root_key(root);
    }
    Ok(builder)
}

async fn attach_store_billing<R: Runtime>(
    app: &AppHandle<R>,
    api_key: &str,
    mut builder: ConfigurationBuilder,
) -> Result<(ConfigurationBuilder, &'static str), Error> {
    if ApiKeyKind::from_api_key(api_key) == ApiKeyKind::TestStore {
        return Ok((builder, "test store"));
    }
    let billing = crate::store_billing(app)?;
    if let Some(is_sandbox) = billing.is_sandbox_environment().await? {
        builder = builder.sandbox_environment(is_sandbox);
    }
    let store = if cfg!(target_os = "android") {
        "play store"
    } else {
        "app store"
    };
    Ok((builder.store_billing(billing), store))
}

async fn settle_initial_identity(
    purchases: &Purchases,
    identity_path: &std::path::Path,
    should_resume_pending: bool,
    persisted_identity: Option<&PersistedIdentity>,
) -> Result<(), Error> {
    if should_resume_pending {
        if let Some(identity) = persisted_identity {
            resume_persisted_identity_transition(purchases, identity_path, identity).await?;
        }
    }
    identity_persistence::persist(
        identity_path,
        &PersistedIdentity::settled(purchases.app_user_id()),
    )
}

#[derive(Debug)]
struct InitialIdentity {
    app_user_id: String,
    should_resume_pending: bool,
}

fn resolve_initial_identity(
    explicit_app_user_id: Option<&str>,
    persisted_identity: Option<&PersistedIdentity>,
) -> Result<InitialIdentity, Error> {
    if let Some(identity) = persisted_identity {
        if let Some(pending_app_user_id) = identity.pending_app_user_id.as_deref() {
            if explicit_app_user_id.is_some_and(|explicit| explicit != pending_app_user_id) {
                return Err(Error::new(
                    ErrorCode::ConfigurationError,
                    "The configured App User ID conflicts with an incomplete durable identity transition. Configure with the pending App User ID or omit it so the transition can resume.",
                ));
            }
            return Ok(InitialIdentity {
                app_user_id: identity.current_app_user_id.clone(),
                should_resume_pending: true,
            });
        }
    }
    Ok(InitialIdentity {
        app_user_id: explicit_app_user_id
            .map(str::to_owned)
            .or_else(|| persisted_identity.map(|identity| identity.current_app_user_id.clone()))
            .unwrap_or_else(revenuecat::generate_anonymous_id),
        should_resume_pending: false,
    })
}

fn configured_with_listener<R: Runtime>(
    app: &AppHandle<R>,
    purchases: Purchases,
    store: &str,
    fingerprint: ConfigureFingerprint,
) -> Configured {
    let handle = app.clone();
    purchases.set_customer_info_listener(move |info| {
        let _ = handle.emit(CUSTOMER_INFO_UPDATED_EVENT, info);
    });
    Configured {
        purchases,
        store: store.to_owned(),
        fingerprint,
    }
}

#[command]
pub(crate) async fn configure<R: Runtime>(
    app: AppHandle<R>,
    options: ConfigureCommandOptions,
) -> Result<SessionInfo, Error> {
    let ConfigureCommandOptions {
        base: options,
        dangerously_allow_test_store_in_release_builds,
    } = options;
    let fingerprint = ConfigureFingerprint::from_options(
        &options,
        dangerously_allow_test_store_in_release_builds,
    )?;
    let sdk_state = app.state::<Sdk>();
    let _configuration_operation = sdk_state.configuration_operation.lock().await;
    if let Some(session) = existing_configuration(&sdk_state, &fingerprint)? {
        return Ok(session);
    }
    let identity_path = persisted_identity_path(&app)?;
    let persisted_identity = identity_persistence::load(&identity_path)?;
    let explicit_app_user_id = options
        .app_user_id
        .as_deref()
        .filter(|app_user_id| !app_user_id.is_empty());
    let initial_identity =
        resolve_initial_identity(explicit_app_user_id, persisted_identity.as_ref())?;
    let builder = base_configuration(
        &options,
        &initial_identity.app_user_id,
        dangerously_allow_test_store_in_release_builds,
    )?;
    let (builder, store) = attach_store_billing(&app, &options.api_key, builder).await?;
    let purchases = Purchases::configure(builder.build()?)?;
    settle_initial_identity(
        &purchases,
        &identity_path,
        initial_identity.should_resume_pending,
        persisted_identity.as_ref(),
    )
    .await?;
    let configured = configured_with_listener(&app, purchases, store, fingerprint);
    let session = info_of_configured(&configured);
    let mut sdk = sdk_state.lock();
    *sdk = Some(configured);
    Ok(session)
}

async fn resume_persisted_identity_transition(
    purchases: &Purchases,
    identity_path: &std::path::Path,
    identity: &PersistedIdentity,
) -> Result<(), Error> {
    let Some(pending_app_user_id) = identity.pending_app_user_id.as_deref() else {
        return Ok(());
    };
    let commit = |app_user_id: &str| {
        identity_persistence::persist(identity_path, &PersistedIdentity::settled(app_user_id))
    };
    match identity.pending_operation() {
        Some(IdentityOperation::LogOut) => {
            purchases
                .resume_log_out_with_identity_transaction(
                    pending_app_user_id,
                    |_, _| Ok(()),
                    commit,
                )
                .await?;
        }
        Some(IdentityOperation::LogIn) | None => {
            purchases
                .log_in_with_identity_transaction(pending_app_user_id, |_, _| Ok(()), commit)
                .await?;
        }
    }
    Ok(())
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

/// Reconciles only native transactions that still need backend posting.
/// This deliberately has different receipt-sharing semantics from the
/// user-initiated `restore` command.
#[command]
pub(crate) async fn sync_pending_purchases(sdk: State<'_, Sdk>) -> Result<CustomerInfo, Error> {
    sdk.purchases()?.sync_pending_purchases().await
}

/// Startup/foreground recovery, including purchases already acknowledged by
/// the store. Identified users still use non-restore receipt semantics.
#[command]
pub(crate) async fn sync_purchases(sdk: State<'_, Sdk>) -> Result<CustomerInfo, Error> {
    sdk.purchases()?.sync_purchases().await
}

#[command]
pub(crate) async fn log_in<R: Runtime>(
    app: AppHandle<R>,
    sdk: State<'_, Sdk>,
    app_user_id: String,
) -> Result<LoginResult, Error> {
    let purchases = sdk.purchases()?;
    let identity_path = persisted_identity_path(&app)?;
    if let Some(identity) = identity_persistence::load(&identity_path)? {
        if let Some(pending_app_user_id) = identity.pending_app_user_id.as_deref() {
            if identity.pending_operation() != Some(IdentityOperation::LogIn)
                || pending_app_user_id != app_user_id
            {
                return Err(pending_identity_operation_error());
            }
        }
    }
    let (customer_info, created) = purchases
        .log_in_with_identity_transaction(
            &app_user_id,
            |current_app_user_id, pending_app_user_id| {
                identity_persistence::persist(
                    &identity_path,
                    &PersistedIdentity::transitioning(
                        current_app_user_id,
                        pending_app_user_id,
                        IdentityOperation::LogIn,
                    ),
                )
            },
            |new_app_user_id| {
                identity_persistence::persist(
                    &identity_path,
                    &PersistedIdentity::settled(new_app_user_id),
                )
            },
        )
        .await?;
    Ok(LoginResult {
        customer_info,
        created,
    })
}

#[command]
pub(crate) async fn log_out<R: Runtime>(
    app: AppHandle<R>,
    sdk: State<'_, Sdk>,
) -> Result<CustomerInfo, Error> {
    let purchases = sdk.purchases()?;
    let identity_path = persisted_identity_path(&app)?;
    if let Some(identity) = identity_persistence::load(&identity_path)? {
        if let Some(pending_app_user_id) = identity.pending_app_user_id.as_deref() {
            if identity.pending_operation() != Some(IdentityOperation::LogOut) {
                return Err(pending_identity_operation_error());
            }
            return purchases
                .resume_log_out_with_identity_transaction(
                    pending_app_user_id,
                    |_, _| Ok(()),
                    |new_app_user_id| {
                        identity_persistence::persist(
                            &identity_path,
                            &PersistedIdentity::settled(new_app_user_id),
                        )
                    },
                )
                .await;
        }
    }
    purchases
        .log_out_with_identity_transaction(
            |current_app_user_id, pending_app_user_id| {
                identity_persistence::persist(
                    &identity_path,
                    &PersistedIdentity::transitioning(
                        current_app_user_id,
                        pending_app_user_id,
                        IdentityOperation::LogOut,
                    ),
                )
            },
            |new_app_user_id| {
                identity_persistence::persist(
                    &identity_path,
                    &PersistedIdentity::settled(new_app_user_id),
                )
            },
        )
        .await
}

fn pending_identity_operation_error() -> Error {
    Error::new(
        ErrorCode::OperationAlreadyInProgressError,
        "A durable App User ID transition is incomplete. Retry the same logIn/logOut operation before starting another identity change.",
    )
}

#[command]
pub(crate) async fn set_email(sdk: State<'_, Sdk>, email: String) -> Result<(), Error> {
    sdk.purchases()?.set_email(email).await
}

/// The store's manage/cancel page for the current user, or `None` if the store
/// doesn't provide one. Open it with `tauri-plugin-opener`; on the App Store /
/// Play Store it lands on the native subscription-management surface.
#[command]
pub(crate) async fn manage_subscriptions(sdk: State<'_, Sdk>) -> Result<Option<String>, Error> {
    Ok(sdk
        .purchases()?
        .get_customer_info(CacheFetchPolicy::default())
        .await?
        .management_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_mode_defaults_to_informational() {
        assert_eq!(
            verification_mode(None).unwrap(),
            EntitlementVerificationMode::Informational
        );
        assert_eq!(
            verification_mode(Some("enforced")).unwrap(),
            EntitlementVerificationMode::Enforced
        );
        assert_eq!(
            verification_mode(Some("disabled")).unwrap(),
            EntitlementVerificationMode::Disabled
        );
    }

    #[test]
    fn unknown_entitlement_verification_mode_is_rejected() {
        let options = ConfigureOptions {
            api_key: "test_key".into(),
            app_user_id: None,
            proxy_url: None,
            entitlement_verification_mode: Some("enfoced".into()),
            verification_root_key: None,
        };

        let parse_error = verification_mode(Some("enfoced")).unwrap_err();
        let Err(fingerprint_error) = ConfigureFingerprint::from_options(&options, false) else {
            panic!("unknown verification mode must not produce a fingerprint");
        };
        let Err(configuration_error) = base_configuration(&options, "user", false) else {
            panic!("unknown verification mode must not produce a configuration");
        };

        for error in [parse_error, fingerprint_error, configuration_error] {
            assert_eq!(error.code, ErrorCode::ConfigurationError);
        }
    }

    #[test]
    fn test_store_release_escape_hatch_is_optional_and_camel_case() {
        let default_options: ConfigureCommandOptions =
            serde_json::from_value(serde_json::json!({ "apiKey": "test_key" })).unwrap();
        assert!(!default_options.dangerously_allow_test_store_in_release_builds);

        let opted_in: ConfigureCommandOptions = serde_json::from_value(serde_json::json!({
            "apiKey": "test_key",
            "dangerouslyAllowTestStoreInReleaseBuilds": true
        }))
        .unwrap();
        assert!(opted_in.dangerously_allow_test_store_in_release_builds);
    }

    #[test]
    fn configure_fingerprint_includes_every_security_sensitive_option() {
        let base = ConfigureOptions {
            api_key: "test_key".to_owned(),
            app_user_id: Some("user-a".to_owned()),
            proxy_url: Some("https://proxy.example".to_owned()),
            entitlement_verification_mode: Some("enforced".to_owned()),
            verification_root_key: Some("root-key".to_owned()),
        };
        let fingerprint = ConfigureFingerprint::from_options(&base, false).unwrap();
        assert_eq!(fingerprint.api_key, "test_key");
        assert_eq!(fingerprint.app_user_id.as_deref(), Some("user-a"));
        assert_eq!(
            fingerprint.proxy_url.as_deref(),
            Some("https://proxy.example")
        );
        assert_eq!(
            fingerprint.entitlement_verification_mode,
            EntitlementVerificationMode::Enforced
        );
        assert_eq!(
            fingerprint.verification_root_key.as_deref(),
            Some("root-key")
        );
        assert!(!fingerprint.dangerously_allow_test_store_in_release_builds);
        assert!(fingerprint != ConfigureFingerprint::from_options(&base, true).unwrap());
    }

    #[test]
    fn configure_options_debug_redacts_api_proxy_and_verification_keys() {
        let options = ConfigureOptions {
            api_key: "appl_secret".into(),
            app_user_id: Some("private-user@example.com".into()),
            proxy_url: Some(
                "https://proxy-user:proxy-password@proxy.example/path?access_token=proxy-query-secret"
                    .into(),
            ),
            entitlement_verification_mode: Some("enforced".into()),
            verification_root_key: Some("secret-root-key".into()),
        };

        let debug = format!("{options:?}");

        assert!(!debug.contains("appl_secret"));
        assert!(!debug.contains("private-user@example.com"));
        assert!(!debug.contains("proxy-user"));
        assert!(!debug.contains("proxy-password"));
        assert!(!debug.contains("proxy.example"));
        assert!(!debug.contains("proxy-query-secret"));
        assert!(!debug.contains("secret-root-key"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn unconfigured_sdk_reports_a_clear_error() {
        let sdk = Sdk::default();
        let error = sdk.purchases().unwrap_err();
        assert_eq!(error.code, ErrorCode::ConfigurationError);
        assert!(!info_of(&sdk.lock()).configured);
    }

    #[test]
    fn pending_login_rejects_a_different_explicit_identity() {
        let persisted =
            PersistedIdentity::transitioning("user-a", "user-b", IdentityOperation::LogIn);

        let error = resolve_initial_identity(Some("user-c"), Some(&persisted)).unwrap_err();

        assert_eq!(error.code, ErrorCode::ConfigurationError);
    }

    #[test]
    fn matching_explicit_identity_resumes_from_the_persisted_current_identity() {
        let persisted =
            PersistedIdentity::transitioning("user-a", "user-b", IdentityOperation::LogIn);

        let initial = resolve_initial_identity(Some("user-b"), Some(&persisted)).unwrap();

        assert_eq!(initial.app_user_id, "user-a");
        assert!(initial.should_resume_pending);
    }

    #[test]
    fn explicit_identity_overrides_a_settled_persisted_identity() {
        let persisted = PersistedIdentity::settled("user-a");

        let initial = resolve_initial_identity(Some("user-b"), Some(&persisted)).unwrap();

        assert_eq!(initial.app_user_id, "user-b");
        assert!(!initial.should_resume_pending);
    }
}
