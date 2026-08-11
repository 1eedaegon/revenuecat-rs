//! Tauri demo for the `revenuecat` crate.
//!
//! On startup the app spawns an in-process `revenuecat-mock` backend on an
//! ephemeral port and configures the SDK with a `test_` (Test Store) API key
//! pointed at it via `proxy_url` — the same wiring the integration tests use,
//! so the whole purchase flow runs with zero external services. Swapping in a
//! real RevenueCat Test Store key is a two-line change (drop the mock, drop
//! `proxy_url`).

use revenuecat::{
    CacheFetchPolicy, Configuration, CustomerInfo, EntitlementVerificationMode, Error, ErrorCode,
    Offerings, PurchaseResult, Purchases,
};
use revenuecat_mock::{test_root_public_key_b64, MockRevenueCat, MockServerHandle};
use tauri::State;

pub struct DemoState {
    purchases: Purchases,
    mock_url: String,
    // Keeps the mock backend alive for the app's lifetime.
    _mock: MockServerHandle,
}

/// Spawns the embedded mock backend and configures the SDK against it.
pub async fn init_demo() -> Result<DemoState, Error> {
    let mock = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .map_err(|e| Error::with_underlying(ErrorCode::ConfigurationError, e.to_string()))?;

    let purchases = Purchases::configure(
        Configuration::builder("test_tauri_demo_key")
            .proxy_url(&mock.url)
            .platform_flavor("tauri", tauri::VERSION)
            // The mock signs with its own test chain; verify every response.
            .entitlement_verification_mode(EntitlementVerificationMode::Informational)
            .verification_root_key(test_root_public_key_b64())
            .build()?,
    )?;

    Ok(DemoState {
        purchases,
        mock_url: mock.url.clone(),
        _mock: mock,
    })
}

#[derive(serde::Serialize)]
pub struct SessionInfo {
    pub app_user_id: String,
    pub is_anonymous: bool,
    pub mock_url: String,
}

#[tauri::command]
fn session_info(state: State<'_, DemoState>) -> SessionInfo {
    SessionInfo {
        app_user_id: state.purchases.app_user_id(),
        is_anonymous: state.purchases.is_anonymous(),
        mock_url: state.mock_url.clone(),
    }
}

#[tauri::command]
async fn get_offerings(state: State<'_, DemoState>) -> Result<Offerings, Error> {
    state.purchases.get_offerings().await
}

#[tauri::command]
async fn get_customer_info(state: State<'_, DemoState>) -> Result<CustomerInfo, Error> {
    state
        .purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
}

#[tauri::command]
async fn purchase(
    state: State<'_, DemoState>,
    package_id: String,
) -> Result<PurchaseResult, Error> {
    let offerings = state.purchases.get_offerings().await?;
    let package = offerings
        .current()
        .and_then(|offering| offering.package(&package_id))
        .ok_or_else(|| {
            Error::new(
                ErrorCode::ProductNotAvailableForPurchaseError,
                format!("Package '{package_id}' not found in the current offering."),
            )
        })?;
    state.purchases.purchase_package(package).await
}

#[tauri::command]
async fn restore(state: State<'_, DemoState>) -> Result<CustomerInfo, Error> {
    state.purchases.restore_purchases().await
}

#[derive(serde::Serialize)]
pub struct LoginResult {
    pub customer_info: CustomerInfo,
    pub created: bool,
}

#[tauri::command]
async fn login(state: State<'_, DemoState>, app_user_id: String) -> Result<LoginResult, Error> {
    let (customer_info, created) = state.purchases.log_in(&app_user_id).await?;
    Ok(LoginResult {
        customer_info,
        created,
    })
}

#[tauri::command]
async fn logout(state: State<'_, DemoState>) -> Result<CustomerInfo, Error> {
    state.purchases.log_out().await
}

/// Builder wiring shared by the real app and the mock-runtime tests.
pub fn handlers<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
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
    tauri::Builder::default()
        .setup(|app| {
            let state = tauri::async_runtime::block_on(init_demo())?;
            tauri::Manager::manage(app, state);
            Ok(())
        })
        .invoke_handler(handlers())
        .run(tauri::generate_context!())
        .expect("error while running the revenuecat-rs demo app");
}
