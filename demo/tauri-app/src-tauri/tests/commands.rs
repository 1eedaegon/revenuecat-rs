//! Tauri IPC tests on the mock runtime: every command is invoked through the
//! real Tauri invoke pipeline (`get_ipc_response`), exercising the same
//! serialization boundary the webview uses — no display server required.

#![allow(clippy::unwrap_used, clippy::panic)]

use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager, WebviewWindow, WebviewWindowBuilder};

fn build_app() -> (App<MockRuntime>, WebviewWindow<MockRuntime>) {
    let app = tauri::test::mock_builder()
        .invoke_handler(revenuecat_tauri_demo::handlers())
        .build(mock_context(noop_assets()))
        .unwrap();
    let state = tauri::async_runtime::block_on(revenuecat_tauri_demo::init_demo()).unwrap();
    app.manage(state);
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    (app, webview)
}

fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    tauri::test::get_ipc_response(
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

#[test]
fn full_purchase_flow_through_tauri_ipc() {
    // Arrange
    let (_app, webview) = build_app();

    // Act / Assert: session starts anonymous against the embedded mock.
    let session = invoke(&webview, "session_info", json!({})).unwrap();
    assert_eq!(session["is_anonymous"], true);
    assert!(session["app_user_id"]
        .as_str()
        .unwrap()
        .starts_with("$RCAnonymousID:"));
    assert!(session["mock_url"]
        .as_str()
        .unwrap()
        .starts_with("http://127.0.0.1:"));

    // Offerings arrive resolved with store products and prices.
    let offerings = invoke(&webview, "get_offerings", json!({})).unwrap();
    assert_eq!(offerings["current_offering_id"], "default");
    let packages = offerings["all"]["default"]["packages"].as_array().unwrap();
    assert_eq!(packages.len(), 3);

    // Purchasing the monthly package grants the `pro` entitlement.
    let result = invoke(&webview, "purchase", json!({"packageId": "$rc_monthly"})).unwrap();
    assert!(result["transaction"]["purchase_token"]
        .as_str()
        .unwrap()
        .starts_with("test_"));
    assert_eq!(
        result["customer_info"]["entitlements"]["all"]["pro"]["is_active"],
        true
    );

    // And customer info reflects it on a fresh fetch.
    let info = invoke(&webview, "get_customer_info", json!({})).unwrap();
    assert_eq!(info["entitlements"]["all"]["pro"]["is_active"], true);
    assert!(info["subscriptions"]["monthly"]["expires_date"]
        .as_str()
        .is_some());
}

#[test]
fn purchase_of_unknown_package_returns_typed_error() {
    let (_app, webview) = build_app();

    let error = invoke(&webview, "purchase", json!({"packageId": "$rc_lifetime"})).unwrap_err();

    assert_eq!(error["code"], "ProductNotAvailableForPurchaseError");
    assert!(error["message"].as_str().unwrap().contains("$rc_lifetime"));
}

#[test]
fn login_and_logout_round_trip_through_ipc() {
    let (_app, webview) = build_app();

    // Purchase anonymously, then attach the history to a named account.
    invoke(&webview, "purchase", json!({"packageId": "$rc_annual"})).unwrap();
    let login = invoke(&webview, "login", json!({"appUserId": "gon"})).unwrap();
    assert_eq!(login["created"], true);
    assert_eq!(
        login["customer_info"]["entitlements"]["all"]["pro"]["is_active"],
        true
    );

    let session = invoke(&webview, "session_info", json!({})).unwrap();
    assert_eq!(session["app_user_id"], "gon");
    assert_eq!(session["is_anonymous"], false);

    // Logout resets to a fresh anonymous user without entitlements.
    let info = invoke(&webview, "logout", json!({})).unwrap();
    assert_eq!(info["entitlements"]["all"].as_object().unwrap().len(), 0);
    let session = invoke(&webview, "session_info", json!({})).unwrap();
    assert_eq!(session["is_anonymous"], true);
}

#[test]
fn restore_via_ipc_refreshes_customer_info() {
    let (_app, webview) = build_app();

    let info = invoke(&webview, "restore", json!({})).unwrap();

    assert!(info["original_app_user_id"]
        .as_str()
        .unwrap()
        .starts_with("$RCAnonymousID:"));
}
