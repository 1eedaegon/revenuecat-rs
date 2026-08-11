//! Tauri demo for the `revenuecat` crate.
//!
//! The demo is configured at runtime from the UI:
//! - no API key -> spawns an in-process `revenuecat-mock` backend and uses a
//!   `test_` key against it (fully offline),
//! - a `test_` key -> talks to the REAL RevenueCat backend with the built-in
//!   simulated Test Store (no store account needed),
//! - an `appl_`/`goog_` key on mobile -> routes purchases through the
//!   platform store via `tauri-plugin-revenuecat` (StoreKit 2 / Play
//!   Billing sandbox).

use std::sync::Mutex;

use revenuecat::{
    ApiKeyKind, CacheFetchPolicy, Configuration, CustomerInfo, EntitlementVerificationMode, Error,
    ErrorCode, Offerings, PurchaseResult, Purchases,
};
use revenuecat_mock::{test_root_public_key_b64, MockRevenueCat, MockServerHandle};
use tauri::{AppHandle, Manager, Runtime, State};

struct Session {
    purchases: Purchases,
    backend: String,
    store: String,
    // Keeps the embedded mock alive for the session's lifetime.
    _mock: Option<MockServerHandle>,
}

#[derive(Default)]
pub struct DemoState {
    session: Mutex<Option<Session>>,
}

#[derive(serde::Serialize)]
pub struct SessionInfo {
    pub configured: bool,
    pub app_user_id: Option<String>,
    pub is_anonymous: Option<bool>,
    /// `"embedded mock"` or `"api.revenuecat.com"`.
    pub backend: Option<String>,
    /// `"test store"`, `"app store"`, or `"play store"`.
    pub store: Option<String>,
}

fn info_of(session: &Option<Session>) -> SessionInfo {
    match session {
        Some(session) => SessionInfo {
            configured: true,
            app_user_id: Some(session.purchases.app_user_id()),
            is_anonymous: Some(session.purchases.is_anonymous()),
            backend: Some(session.backend.clone()),
            store: Some(session.store.clone()),
        },
        None => SessionInfo {
            configured: false,
            app_user_id: None,
            is_anonymous: None,
            backend: None,
            store: None,
        },
    }
}

fn purchases_of(state: &State<'_, DemoState>) -> Result<Purchases, Error> {
    state
        .session
        .lock()
        .expect("session lock poisoned")
        .as_ref()
        .map(|session| session.purchases.clone())
        .ok_or_else(|| {
            Error::new(
                ErrorCode::ConfigurationError,
                "Not configured yet — enter an API key or start with the embedded mock.",
            )
        })
}

async fn build_session<R: Runtime>(
    app: &AppHandle<R>,
    api_key: Option<String>,
    app_user_id: Option<String>,
) -> Result<Session, Error> {
    let api_key = api_key
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty());
    let app_user_id = app_user_id
        .map(|u| u.trim().to_owned())
        .filter(|u| !u.is_empty());

    let mut builder = match &api_key {
        // Offline mode: embedded mock + its test signing chain.
        None => {
            let mock = MockRevenueCat::with_default_catalog()
                .spawn()
                .await
                .map_err(|e| {
                    Error::with_underlying(ErrorCode::ConfigurationError, e.to_string())
                })?;
            let builder = Configuration::builder("test_tauri_demo_key")
                .proxy_url(&mock.url)
                .verification_root_key(test_root_public_key_b64());
            let mut builder = builder;
            if let Some(user) = &app_user_id {
                builder = builder.app_user_id(user);
            }
            let purchases = Purchases::configure(
                builder
                    .platform_flavor("tauri", tauri::VERSION)
                    .entitlement_verification_mode(EntitlementVerificationMode::Informational)
                    .build()?,
            )?;
            return Ok(Session {
                purchases,
                backend: "embedded mock".into(),
                store: "test store".into(),
                _mock: Some(mock),
            });
        }
        Some(key) => Configuration::builder(key.clone()),
    };

    if let Some(user) = &app_user_id {
        builder = builder.app_user_id(user);
    }
    builder = builder
        .platform_flavor("tauri", tauri::VERSION)
        .entitlement_verification_mode(EntitlementVerificationMode::Informational);

    let kind = ApiKeyKind::from_api_key(api_key.as_deref().unwrap_or_default());
    let (builder, store) = match kind {
        // Real backend + built-in simulated Test Store: no native store.
        ApiKeyKind::TestStore => (builder, "test store".to_owned()),
        // Real store keys route purchases through the platform store shim,
        // available only in native-store builds.
        _ => configure_native_store(app, builder)?,
    };

    Ok(Session {
        purchases: Purchases::configure(builder.build()?)?,
        backend: "api.revenuecat.com".into(),
        store,
        _mock: None,
    })
}

/// Attaches the native store (StoreKit 2 / Play Billing) in `native-store`
/// builds; otherwise reports that this demo build cannot use real-store keys.
#[cfg(feature = "native-store")]
fn configure_native_store<R: Runtime>(
    app: &AppHandle<R>,
    builder: revenuecat::ConfigurationBuilder,
) -> Result<(revenuecat::ConfigurationBuilder, String), Error> {
    let billing = tauri_plugin_revenuecat::store_billing(app)?;
    let store = if cfg!(target_os = "android") {
        "play store"
    } else {
        "app store"
    };
    Ok((builder.store_billing(billing), store.to_owned()))
}

#[cfg(not(feature = "native-store"))]
fn configure_native_store<R: Runtime>(
    _app: &AppHandle<R>,
    _builder: revenuecat::ConfigurationBuilder,
) -> Result<(revenuecat::ConfigurationBuilder, String), Error> {
    Err(Error::new(
        ErrorCode::ConfigurationError,
        "This demo build has no native store. Use a test_ key (real backend, \
         simulated Test Store) or leave the key empty for the embedded mock. \
         Rebuild with --features native-store for StoreKit / Play Billing.",
    ))
}

// -- Commands ---------------------------------------------------------------

#[tauri::command]
async fn configure_demo<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, DemoState>,
    api_key: Option<String>,
    app_user_id: Option<String>,
) -> Result<SessionInfo, Error> {
    let session = build_session(&app, api_key, app_user_id).await?;
    let mut guard = state.session.lock().expect("session lock poisoned");
    *guard = Some(session);
    Ok(info_of(&guard))
}

#[tauri::command]
fn session_info(state: State<'_, DemoState>) -> SessionInfo {
    info_of(&state.session.lock().expect("session lock poisoned"))
}

#[tauri::command]
async fn get_offerings(state: State<'_, DemoState>) -> Result<Offerings, Error> {
    purchases_of(&state)?.get_offerings().await
}

#[tauri::command]
async fn get_customer_info(state: State<'_, DemoState>) -> Result<CustomerInfo, Error> {
    purchases_of(&state)?
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
}

#[tauri::command]
async fn purchase(
    state: State<'_, DemoState>,
    package_id: String,
) -> Result<PurchaseResult, Error> {
    let purchases = purchases_of(&state)?;
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

#[tauri::command]
async fn restore(state: State<'_, DemoState>) -> Result<CustomerInfo, Error> {
    purchases_of(&state)?.restore_purchases().await
}

#[derive(serde::Serialize)]
pub struct LoginResult {
    pub customer_info: CustomerInfo,
    pub created: bool,
}

#[tauri::command]
async fn login(state: State<'_, DemoState>, app_user_id: String) -> Result<LoginResult, Error> {
    let (customer_info, created) = purchases_of(&state)?.log_in(&app_user_id).await?;
    Ok(LoginResult {
        customer_info,
        created,
    })
}

#[tauri::command]
async fn logout(state: State<'_, DemoState>) -> Result<CustomerInfo, Error> {
    purchases_of(&state)?.log_out().await
}

/// Builder wiring shared by the real app and the mock-runtime tests.
pub fn handlers<R: Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        configure_demo,
        session_info,
        get_offerings,
        get_customer_info,
        purchase,
        restore,
        login,
        logout
    ]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(feature = "native-store")]
    let builder = builder.plugin(tauri_plugin_revenuecat::init());
    builder
        .setup(|app| {
            app.manage(DemoState::default());
            Ok(())
        })
        .invoke_handler(handlers())
        .run(tauri::generate_context!())
        .expect("error while running the revenuecat-rs demo app");
}
