const COMMANDS: &[&str] = &[
    // Required by @tauri-apps/api addPluginListener for native events.
    "register_listener",
    "remove_listener",
    // App-facing SDK commands, registered in the invoke handler.
    "configure",
    "session_info",
    "get_offerings",
    "get_customer_info",
    "purchase_package",
    "restore",
    "sync_pending_purchases",
    "sync_purchases",
    "log_in",
    "log_out",
    "set_email",
    "manage_subscriptions",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
