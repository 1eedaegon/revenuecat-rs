const COMMANDS: &[&str] = &[
    "query_products",
    "purchase",
    "query_purchases",
    "finish_transaction",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
