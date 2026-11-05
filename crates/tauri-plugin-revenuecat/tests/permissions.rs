const DEFAULT_PERMISSION: &str = include_str!("../permissions/default.toml");
const BUILD_SCRIPT: &str = include_str!("../build.rs");

#[test]
fn default_permission_supports_native_event_listeners() {
    for command in ["register_listener", "remove_listener"] {
        let permission = command.replace('_', "-");
        assert!(
            BUILD_SCRIPT.contains(&format!("\"{command}\"")),
            "{command} must be registered with the Tauri plugin builder"
        );
        assert!(
            DEFAULT_PERMISSION.contains(&format!("\"allow-{permission}\"")),
            "{command} must be allowed for addPluginListener"
        );
    }
}

#[test]
fn default_permission_excludes_internal_native_billing_commands() {
    for (rust_command, permission_command) in [
        ("is_sandbox_environment", "is-sandbox-environment"),
        ("query_products", "query-products"),
        ("purchase", "purchase"),
        ("query_purchases", "query-purchases"),
        ("query_pending_purchases", "query-pending-purchases"),
        ("finish_transaction", "finish-transaction"),
    ] {
        assert!(
            !BUILD_SCRIPT.contains(&format!("\"{rust_command}\"")),
            "internal native command {rust_command} must not generate a webview permission"
        );
        assert!(
            !DEFAULT_PERMISSION.contains(&format!("\"allow-{permission_command}\"")),
            "internal native command {rust_command} must not be webview-accessible by default"
        );
    }
}
