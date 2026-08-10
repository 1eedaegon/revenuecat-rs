//! Diagnostics end-to-end: entries queue during SDK operations and flush to
//! the diagnostics endpoint with the exact Android wire shape.

#![allow(clippy::unwrap_used, clippy::panic)]

use revenuecat::{CacheFetchPolicy, Configuration, EntitlementVerificationMode, Purchases};
use revenuecat_mock::{test_root_public_key_b64, MockRevenueCat};

#[tokio::test]
async fn diagnostics_entries_flush_with_android_wire_shape() {
    // Arrange: diagnostics on, pointed at the mock (dedicated-host stand-in).
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("test_diag_key")
            .proxy_url(&server.url)
            .diagnostics_enabled(true)
            .diagnostics_url(&server.url)
            .entitlement_verification_mode(EntitlementVerificationMode::Informational)
            .verification_root_key(test_root_public_key_b64())
            .app_user_id("gon")
            .build()
            .unwrap(),
    )
    .unwrap();

    // Act: a few tracked operations, then an explicit flush.
    purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
        .unwrap();
    let offerings = purchases.get_offerings().await.unwrap();
    let monthly = offerings.current().unwrap().monthly().unwrap();
    purchases.purchase_package(monthly).await.unwrap();
    purchases.flush_diagnostics().await.unwrap();

    // Assert: the wrapper and entry shapes match DiagnosticsEntry exactly.
    let posts = server.received_on("/v1/diagnostics");
    assert_eq!(posts.len(), 1);
    assert!(posts[0].body.as_ref().unwrap()["entries"].is_array());

    let entries = server.state.diagnostics();
    let names: Vec<&str> = entries.iter().filter_map(|e| e["name"].as_str()).collect();
    assert!(names.contains(&"http_request_performed"));
    assert!(
        names.contains(&"customer_info_verification_result"),
        "verified customer info must record its verification result"
    );

    let http_entry = entries
        .iter()
        .find(|e| e["name"] == "http_request_performed")
        .unwrap();
    assert_eq!(http_entry["version"], 1);
    assert!(http_entry["id"].as_str().is_some());
    assert!(http_entry["app_session_id"].as_str().is_some());
    assert!(http_entry["timestamp"].as_str().unwrap().ends_with('Z'));
    let properties = &http_entry["properties"];
    assert!(properties["endpoint_name"].as_str().is_some());
    assert_eq!(properties["successful"], true);
    assert_eq!(properties["response_code"], 200);
    assert_eq!(properties["verification_result"], "VERIFIED");

    // Endpoint coverage: customer info, offerings, products join, receipt.
    let endpoints: Vec<&str> = entries
        .iter()
        .filter(|e| e["name"] == "http_request_performed")
        .filter_map(|e| e["properties"]["endpoint_name"].as_str())
        .collect();
    assert!(endpoints.contains(&"get_customer"));
    assert!(endpoints.contains(&"get_offerings"));
    assert!(endpoints.contains(&"post_receipt"));

    // A second flush with nothing queued posts nothing.
    purchases.flush_diagnostics().await.unwrap();
    assert_eq!(server.received_on("/v1/diagnostics").len(), 1);
}

#[tokio::test]
async fn diagnostics_disabled_records_and_posts_nothing() {
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("test_diag_key")
            .proxy_url(&server.url)
            .diagnostics_url(&server.url)
            .app_user_id("gon")
            .build()
            .unwrap(),
    )
    .unwrap();

    purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
        .unwrap();
    purchases.flush_diagnostics().await.unwrap();

    assert!(server.received_on("/v1/diagnostics").is_empty());
}

#[tokio::test]
async fn a_full_batch_auto_flushes_without_an_explicit_call() {
    // Arrange
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("test_diag_key")
            .proxy_url(&server.url)
            .diagnostics_enabled(true)
            .diagnostics_url(&server.url)
            .app_user_id("auto")
            .build()
            .unwrap(),
    )
    .unwrap();

    // Act: 200 tracked requests reach the batch threshold; the spawned
    // background flush stands in for Android's 200 KB file trigger.
    for _ in 0..200 {
        purchases
            .get_customer_info(CacheFetchPolicy::FetchCurrent)
            .await
            .unwrap();
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Assert: entries were posted without flush_diagnostics().
    assert!(
        !server.received_on("/v1/diagnostics").is_empty(),
        "auto-flush must post once a full batch accumulates"
    );
}
