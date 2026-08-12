const COMMANDS: &[&str] = &[
    // Native store bridge, invoked internally over the mobile plugin channel.
    "query_products",
    "purchase",
    "query_purchases",
    "finish_transaction",
    // App-facing SDK commands, registered in the invoke handler.
    "configure",
    "session_info",
    "get_offerings",
    "get_customer_info",
    "purchase_package",
    "restore",
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
