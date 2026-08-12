//! Tauri IPC tests on the mock runtime: every command is invoked through the
//! real Tauri invoke pipeline (`get_ipc_response`), exercising the same
//! serialization + ACL boundary the webview uses — no display server required.
//!
//! The SDK lives in `tauri-plugin-revenuecat`, so these drive `plugin:revenuecat|*`
//! commands. `start_mock` (the demo's own command) spawns the embedded backend;
//! `configure` then points the plugin at it. The app is built from the real
//! context (`generate_context!`) so the `revenuecat:default` capability grants
//! the plugin commands through the ACL.

#![allow(clippy::unwrap_used, clippy::panic)]

use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{App, WebviewWindow, WebviewWindowBuilder};

fn build_app() -> (App<MockRuntime>, WebviewWindow<MockRuntime>) {
    let app = revenuecat_tauri_demo::build(mock_builder())
        .build(tauri::generate_context!())
        .unwrap();
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    (app, webview)
}

fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    get_ipc_response(
        webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            // The local-origin URL differs per OS: `tauri://localhost` on
            // macOS/Linux, `http://tauri.localhost` on Windows. A non-local
            // origin would be rejected by the Tauri v2 ACL.
            url: if cfg!(windows) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap())
}

/// Spawns the embedded mock and configures the plugin against it, returning the
/// session info. `app_user_id` is optional (anonymous when absent).
fn configure_mock(webview: &WebviewWindow<MockRuntime>, app_user_id: Option<&str>) -> Value {
    let mock = invoke(webview, "start_mock", json!({})).unwrap();
    invoke(
        webview,
        "plugin:revenuecat|configure",
        json!({
            "options": {
                "apiKey": mock["apiKey"],
                "proxyUrl": mock["proxyUrl"],
                "verificationRootKey": mock["verificationRootKey"],
                "appUserId": app_user_id,
            }
        }),
    )
    .unwrap()
}

#[test]
fn configure_selects_the_embedded_mock_backend() {
    let (_app, webview) = build_app();

    // Before configuration, commands report a configuration error.
    let error = invoke(&webview, "plugin:revenuecat|get_offerings", json!({})).unwrap_err();
    assert_eq!(error["code"], "ConfigurationError");

    // Configuring against the mock reports a Test Store session.
    let session = configure_mock(&webview, None);
    assert_eq!(session["configured"], true);
    assert_eq!(session["store"], "test store");
    assert_eq!(session["isAnonymous"], true);
    assert!(session["appUserId"]
        .as_str()
        .unwrap()
        .starts_with("$RCAnonymousID:"));
}

#[test]
fn full_purchase_flow_through_tauri_ipc() {
    let (_app, webview) = build_app();
    configure_mock(&webview, Some("gon"));

    // Offerings arrive resolved with store products and prices.
    let offerings = invoke(&webview, "plugin:revenuecat|get_offerings", json!({})).unwrap();
    let packages = offerings["all"]["default"]["packages"].as_array().unwrap();
    assert_eq!(packages.len(), 3);

    // Purchasing the monthly package grants the `pro` entitlement.
    let result = invoke(
        &webview,
        "plugin:revenuecat|purchase_package",
        json!({ "packageId": "$rc_monthly" }),
    )
    .unwrap();
    assert!(result["transaction"]["purchase_token"]
        .as_str()
        .unwrap()
        .starts_with("test_"));
    assert_eq!(
        result["customer_info"]["entitlements"]["all"]["pro"]["is_active"],
        true
    );

    // And customer info reflects it on a fresh fetch.
    let info = invoke(&webview, "plugin:revenuecat|get_customer_info", json!({})).unwrap();
    assert_eq!(info["entitlements"]["all"]["pro"]["is_active"], true);
    assert!(info["subscriptions"]["monthly"]["expires_date"]
        .as_str()
        .is_some());
}

#[test]
fn purchase_of_unknown_package_returns_typed_error() {
    let (_app, webview) = build_app();
    configure_mock(&webview, None);

    let error = invoke(
        &webview,
        "plugin:revenuecat|purchase_package",
        json!({ "packageId": "$rc_lifetime" }),
    )
    .unwrap_err();

    assert_eq!(error["code"], "ProductNotAvailableForPurchaseError");
    assert!(error["message"].as_str().unwrap().contains("$rc_lifetime"));
}

#[test]
fn login_and_logout_round_trip_through_ipc() {
    let (_app, webview) = build_app();
    configure_mock(&webview, None);

    // Purchase anonymously, then attach the history to a named account.
    invoke(
        &webview,
        "plugin:revenuecat|purchase_package",
        json!({ "packageId": "$rc_annual" }),
    )
    .unwrap();
    let login = invoke(
        &webview,
        "plugin:revenuecat|log_in",
        json!({ "appUserId": "gon" }),
    )
    .unwrap();
    assert_eq!(login["created"], true);
    assert_eq!(
        login["customerInfo"]["entitlements"]["all"]["pro"]["is_active"],
        true
    );

    let session = invoke(&webview, "plugin:revenuecat|session_info", json!({})).unwrap();
    assert_eq!(session["appUserId"], "gon");
    assert_eq!(session["isAnonymous"], false);

    // Logout resets to a fresh anonymous user without entitlements.
    let info = invoke(&webview, "plugin:revenuecat|log_out", json!({})).unwrap();
    assert_eq!(info["entitlements"]["all"].as_object().unwrap().len(), 0);
    let session = invoke(&webview, "plugin:revenuecat|session_info", json!({})).unwrap();
    assert_eq!(session["isAnonymous"], true);
}

#[test]
fn restore_via_ipc_refreshes_customer_info() {
    let (_app, webview) = build_app();
    configure_mock(&webview, None);

    let info = invoke(&webview, "plugin:revenuecat|restore", json!({})).unwrap();

    assert!(info["original_app_user_id"]
        .as_str()
        .unwrap()
        .starts_with("$RCAnonymousID:"));
}
