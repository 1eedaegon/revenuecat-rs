//! Tauri demo for the `revenuecat` crate.
//!
//! The SDK runs behind `tauri-plugin-revenuecat`: the webview drives it with
//! `plugin:revenuecat|*` commands (via the typed `tauri-plugin-revenuecat-api`
//! bindings). This crate only adds `start_mock`, which spawns an in-process
//! `revenuecat-mock` and returns how to point the plugin's `configure` at it,
//! so the demo runs fully offline. With a `test_`/`appl_`/`goog_` key the
//! frontend configures the plugin directly.

use std::sync::Mutex;

use revenuecat_mock::{test_root_public_key_b64, MockRevenueCat, MockServerHandle};
use tauri::{Runtime, State};

/// Keeps the embedded mock alive for the app's lifetime.
#[derive(Default)]
pub struct MockState {
    handle: Mutex<Option<MockServerHandle>>,
}

/// How to connect the plugin's `configure` to the embedded mock backend.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MockInfo {
    /// The `test_` key to configure the plugin with.
    pub api_key: String,
    /// Point the plugin's `configure` here (like `Purchases.proxyURL`).
    pub proxy_url: String,
    /// The mock's Ed25519 root key, for Trusted Entitlements verification.
    pub verification_root_key: String,
}

/// Spawns the embedded mock backend and returns how to connect the SDK to it.
#[tauri::command]
async fn start_mock(state: State<'_, MockState>) -> Result<MockInfo, String> {
    let mock = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .map_err(|e| e.to_string())?;
    let proxy_url = mock.url.clone();
    *state.handle.lock().expect("mock lock poisoned") = Some(mock);
    Ok(MockInfo {
        api_key: "test_tauri_demo_key".into(),
        proxy_url,
        verification_root_key: test_root_public_key_b64(),
    })
}

/// Builder wiring shared by the real app and the mock-runtime tests: register
/// the plugin and the demo's `start_mock` command. `manage` (not `setup`)
/// registers state at build time, so it's present in tests that don't `run()`.
pub fn build<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .plugin(tauri_plugin_revenuecat::init())
        .manage(MockState::default())
        .invoke_handler(tauri::generate_handler![start_mock])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    build(tauri::Builder::default())
        .run(tauri::generate_context!())
        .expect("error while running the revenuecat-rs demo app");
}
