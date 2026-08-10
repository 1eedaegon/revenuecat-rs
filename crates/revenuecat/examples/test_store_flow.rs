//! Runs the full Test Store purchase flow against the in-process mock
//! backend and prints each step — the same flow the Tauri demo drives.
//!
//! ```sh
//! cargo run -p revenuecat --example test_store_flow
//! # optionally dump every response as JSON:
//! cargo run -p revenuecat --example test_store_flow -- /tmp/flow.json
//! ```

#![allow(clippy::unwrap_used)]

use revenuecat::{CacheFetchPolicy, Configuration, Purchases};
use revenuecat_mock::MockRevenueCat;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mock = MockRevenueCat::with_default_catalog().spawn().await?;
    println!("mock backend      {}", mock.url);

    let purchases = Purchases::configure(
        Configuration::builder("test_example_key")
            .proxy_url(&mock.url)
            .platform_flavor("tauri", "2.0")
            .build()?,
    )?;
    println!("app user id       {}", purchases.app_user_id());

    let offerings = purchases.get_offerings().await?;
    let current = offerings.current().expect("current offering");
    println!(
        "offering          {} ({} packages)",
        current.identifier,
        current.packages.len()
    );
    for package in &current.packages {
        println!(
            "  {:<12} {:<12} {}",
            package.identifier,
            package.store_product.identifier,
            package.store_product.price.formatted()
        );
    }

    let before = purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await?;

    let monthly = current.monthly().expect("monthly package");
    let result = purchases.purchase_package(monthly).await?;
    println!(
        "purchase          token={}",
        result.transaction.purchase_token
    );

    let coins = current.package("coins").expect("coins package");
    let coins_result = purchases.purchase_package(coins).await?;

    let info = purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await?;
    println!(
        "entitlement pro   active={}",
        info.entitlements.is_active("pro")
    );
    println!("subscriptions     {:?}", info.active_subscriptions());
    println!(
        "one-time          {} transaction(s)",
        info.non_subscription_transactions.len()
    );

    if let Some(path) = std::env::args().nth(1) {
        let session = json!({
            "app_user_id": purchases.app_user_id(),
            "is_anonymous": purchases.is_anonymous(),
            "mock_url": mock.url,
        });
        let dump = json!({
            "session": session,
            "offerings": offerings,
            "customer_info_before": before,
            "purchase_result": result,
            "coins_result": coins_result,
            "customer_info_after": info,
        });
        std::fs::write(&path, serde_json::to_string_pretty(&dump)?)?;
        println!("dumped responses  {path}");
    }
    Ok(())
}
