//! End-to-end integration tests: the real SDK stack (facade -> backend ->
//! HTTP client -> ETag manager -> simulated Test Store) against the
//! `revenuecat-mock` server, mirroring how purchases-js tests against MSW —
//! including request-spy assertions on exact paths, headers, and bodies.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::time::Duration;

use revenuecat::{
    CacheFetchPolicy, Configuration, ErrorCode, ProductType, Purchases, Store, StoreProduct,
};
use revenuecat_mock::{MockRevenueCat, MockServerHandle};

const TEST_API_KEY: &str = "test_integration_key";

async fn spawn_mock() -> MockServerHandle {
    MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .expect("mock server should bind")
}

fn configure(server: &MockServerHandle, app_user_id: Option<&str>) -> Purchases {
    let mut builder = Configuration::builder(TEST_API_KEY)
        .proxy_url(&server.url)
        .platform_flavor("tauri", "2.0")
        .http_timeout(Duration::from_secs(5));
    if let Some(id) = app_user_id {
        builder = builder.app_user_id(id);
    }
    Purchases::configure(builder.build().unwrap()).unwrap()
}

#[tokio::test]
async fn full_purchase_flow_grants_entitlement() {
    // Arrange
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("gon"));

    // Act: offerings -> purchase the monthly package -> customer info.
    let offerings = purchases.get_offerings().await.unwrap();
    let current = offerings.current().expect("current offering");
    let monthly = current.monthly().expect("monthly package");
    assert_eq!(monthly.store_product.price.formatted(), "$3.00");

    let result = purchases.purchase_package(monthly).await.unwrap();

    // Assert
    assert_eq!(result.transaction.store, Store::TestStore);
    assert!(result.transaction.purchase_token.starts_with("test_"));
    assert!(result.customer_info.entitlements.is_active("pro"));
    assert!(result
        .customer_info
        .active_subscriptions()
        .contains("monthly"));

    let info = purchases
        .get_customer_info(CacheFetchPolicy::CacheOnly)
        .await
        .unwrap();
    assert!(
        info.entitlements.is_active("pro"),
        "purchase result must be cached"
    );
}

#[tokio::test]
async fn receipt_body_matches_official_wire_format() {
    // Arrange
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("wire-check"));

    // Act
    let offerings = purchases.get_offerings().await.unwrap();
    let monthly = offerings.current().unwrap().monthly().unwrap();
    purchases.purchase_package(monthly).await.unwrap();

    // Assert: exact body fields posted to /v1/receipts.
    let receipts = server.received_on("/v1/receipts");
    assert_eq!(receipts.len(), 1);
    let body = receipts[0].body.as_ref().unwrap();
    assert!(body["fetch_token"].as_str().unwrap().starts_with("test_"));
    assert_eq!(body["app_user_id"], "wire-check");
    assert_eq!(body["product_ids"][0], "monthly");
    assert_eq!(body["platform_product_ids"][0]["product_id"], "monthly");
    assert_eq!(body["is_restore"], false);
    assert_eq!(body["observer_mode"], false);
    assert_eq!(body["purchase_completed_by"], "revenuecat");
    assert_eq!(body["initiation_source"], "purchase");
    assert_eq!(body["payload_version"], 1);
    assert_eq!(body["price"], 3.0);
    assert_eq!(body["currency"], "USD");
    assert_eq!(body["normal_duration"], "P1M");
    assert_eq!(body["presented_offering_identifier"], "default");

    // And protocol headers on the same request.
    let headers = &receipts[0].headers;
    assert_eq!(headers["authorization"], format!("Bearer {TEST_API_KEY}"));
    assert_eq!(headers["x-platform-flavor"], "tauri");
    assert!(headers.contains_key("x-platform"));
    assert!(headers.contains_key("x-version"));
}

#[tokio::test]
async fn consumable_purchase_records_non_subscription() {
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("consumer"));

    let offerings = purchases.get_offerings().await.unwrap();
    let coins = offerings.current().unwrap().package("coins").unwrap();
    assert_eq!(coins.store_product.product_type, ProductType::Consumable);

    let result = purchases.purchase_package(coins).await.unwrap();

    assert_eq!(result.customer_info.non_subscription_transactions.len(), 1);
    assert_eq!(
        result.customer_info.non_subscription_transactions[0].product_identifier,
        "coins_100"
    );
}

#[tokio::test]
async fn anonymous_id_is_generated_and_percent_encoded_in_paths() {
    // Arrange: no app_user_id -> anonymous.
    let server = spawn_mock().await;
    let purchases = configure(&server, None);

    // Act
    assert!(purchases.is_anonymous());
    purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
        .unwrap();

    // Assert: `$RCAnonymousID:` must be encoded as %24RCAnonymousID%3A.
    let requests = server.received_on("/v1/subscribers/%24RCAnonymousID%3A");
    assert_eq!(
        requests.len(),
        1,
        "anonymous id must be percent-encoded in the URL path"
    );
}

#[tokio::test]
async fn etag_304_replays_cached_offerings_body() {
    // Arrange: TTL 0 forces a network hit on every get_offerings call.
    let server = spawn_mock().await;
    let purchases = Purchases::configure(
        Configuration::builder(TEST_API_KEY)
            .proxy_url(&server.url)
            .app_user_id("etag-user")
            .cache_ttl(Duration::ZERO)
            .build()
            .unwrap(),
    )
    .unwrap();

    // Act: first call caches body+etag, second sends the etag and gets 304.
    let first = purchases.get_offerings().await.unwrap();
    let second = purchases.get_offerings().await.unwrap();

    // Assert
    assert_eq!(first.current_offering_id, second.current_offering_id);
    let offerings_requests = server.received_on("/v1/subscribers/etag-user/offerings");
    assert_eq!(offerings_requests.len(), 2);
    assert_eq!(offerings_requests[0].headers["x-revenuecat-etag"], "");
    let second_etag = &offerings_requests[1].headers["x-revenuecat-etag"];
    assert!(
        !second_etag.is_empty(),
        "second request must carry the cached ETag"
    );
}

#[tokio::test]
async fn login_aliases_anonymous_purchases_and_reports_created() {
    // Arrange: anonymous user purchases first.
    let server = spawn_mock().await;
    let purchases = configure(&server, None);
    let offerings = purchases.get_offerings().await.unwrap();
    let monthly = offerings.current().unwrap().monthly().unwrap();
    purchases.purchase_package(monthly).await.unwrap();

    // Act: identify as "gon".
    let (info, created) = purchases.log_in("gon").await.unwrap();

    // Assert: new identity, created=true (HTTP 201), purchases carried over.
    assert!(created);
    assert_eq!(purchases.app_user_id(), "gon");
    assert!(!purchases.is_anonymous());
    assert!(info.entitlements.is_active("pro"));

    // Logging into an existing user reports created=false (HTTP 200).
    let other = configure(&server, None);
    let (_, created_again) = other.log_in("gon").await.unwrap();
    assert!(!created_again);
}

#[tokio::test]
async fn log_out_resets_to_anonymous_and_errors_when_already_anonymous() {
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("gon"));
    purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
        .unwrap();

    let info = purchases.log_out().await.unwrap();
    assert!(purchases.is_anonymous());
    assert!(info.entitlements.active().is_empty());

    let err = purchases.log_out().await.unwrap_err();
    assert_eq!(err.code, ErrorCode::LogOutWithAnonymousUserError);
}

#[tokio::test]
async fn unknown_product_maps_backend_7662_to_unsupported_error() {
    // Arrange: a product the backend has never heard of.
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("gon"));
    let ghost = StoreProduct {
        identifier: "ghost".into(),
        product_type: ProductType::Subscription,
        title: "Ghost".into(),
        description: None,
        price: revenuecat::Price {
            amount_micros: 1,
            currency: "USD".into(),
        },
        subscription_period: Some("P1M".into()),
        trial: None,
        intro_price: None,
    };

    // Act
    let err = purchases.purchase_product(&ghost).await.unwrap_err();

    // Assert: backend {"code": 7662} -> UnsupportedError, code preserved.
    assert_eq!(err.code, ErrorCode::UnsupportedError);
    assert_eq!(err.backend_code, Some(7662));
}

#[tokio::test]
async fn invalid_email_attribute_maps_to_invalid_subscriber_attributes() {
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("gon"));

    let err = purchases.set_email("not-an-email").await.unwrap_err();

    assert_eq!(err.code, ErrorCode::InvalidSubscriberAttributesError);
    assert_eq!(err.backend_code, Some(7263));
    assert!(err
        .underlying
        .as_deref()
        .unwrap_or_default()
        .contains("$email"));
}

#[tokio::test]
async fn valid_attributes_are_posted_with_updated_at_ms() {
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("gon"));

    purchases
        .set_attributes(BTreeMap::from([
            ("$email".to_owned(), Some("gon@example.com".to_owned())),
            ("plan_intent".to_owned(), Some("pro".to_owned())),
        ]))
        .await
        .unwrap();

    let posted = server.received_on("/v1/subscribers/gon/attributes");
    assert_eq!(posted.len(), 1);
    let body = posted[0].body.as_ref().unwrap();
    assert_eq!(body["attributes"]["$email"]["value"], "gon@example.com");
    assert!(body["attributes"]["plan_intent"]["updated_at_ms"].is_i64());
}

#[tokio::test]
async fn empty_attributes_are_rejected_client_side() {
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("gon"));

    let err = purchases.set_attributes(BTreeMap::new()).await.unwrap_err();

    assert_eq!(err.code, ErrorCode::EmptySubscriberAttributesError);
    assert!(server
        .received_on("/v1/subscribers/gon/attributes")
        .is_empty());
}

#[tokio::test]
async fn cache_fetch_policies_behave_like_the_official_sdk() {
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("gon"));

    // CacheOnly before any fetch -> CustomerInfoError.
    let err = purchases
        .get_customer_info(CacheFetchPolicy::CacheOnly)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::CustomerInfoError);

    // FetchCurrent populates the cache; CacheOnly then succeeds without I/O.
    purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
        .unwrap();
    let before = server.received_on("/v1/subscribers/gon").len();
    purchases
        .get_customer_info(CacheFetchPolicy::CacheOnly)
        .await
        .unwrap();
    purchases
        .get_customer_info(CacheFetchPolicy::NotStaleCachedOrFetched)
        .await
        .unwrap();
    let after = server.received_on("/v1/subscribers/gon").len();
    assert_eq!(
        before, after,
        "fresh cache must not trigger network fetches"
    );
}

#[tokio::test]
async fn restore_without_owned_transactions_refreshes_customer_info() {
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("gon"));

    let info = purchases.restore_purchases().await.unwrap();

    assert_eq!(info.original_app_user_id, "gon");
    assert!(server.received_on("/v1/receipts").is_empty());
    assert_eq!(server.received_on("/v1/subscribers/gon").len(), 1);
}

#[tokio::test]
async fn virtual_currencies_fetch_and_cache() {
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("gon"));

    // First call hits the network; the fresh cache then serves the second.
    let currencies = purchases.get_virtual_currencies().await.unwrap();
    assert_eq!(currencies.balance("GLD"), 100);
    assert_eq!(currencies.get("GLD").unwrap().name, "Gold");

    purchases.get_virtual_currencies().await.unwrap();
    assert_eq!(
        server
            .received_on("/v1/subscribers/gon/virtual_currencies")
            .len(),
        1,
        "second read must come from the cache"
    );

    // Invalidation forces a refetch, mirroring invalidateVirtualCurrenciesCache.
    purchases.invalidate_virtual_currencies_cache();
    assert!(purchases.cached_virtual_currencies().is_none());
    purchases.get_virtual_currencies().await.unwrap();
    assert_eq!(
        server
            .received_on("/v1/subscribers/gon/virtual_currencies")
            .len(),
        2
    );
}

#[tokio::test]
async fn missing_store_billing_for_non_test_key_is_a_configuration_error() {
    let err =
        Purchases::configure(Configuration::builder("goog_real_key").build().unwrap()).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigurationError);
}

#[tokio::test]
async fn receipt_posts_retry_on_429_until_success() {
    // Arrange: the next two receipt posts are rate limited.
    let server = spawn_mock().await;
    server.rate_limit_receipts(2);
    let purchases = configure(&server, Some("retry-user"));

    // Act: the purchase must survive two 429s via the retry backoff.
    let offerings = purchases.get_offerings().await.unwrap();
    let monthly = offerings.current().unwrap().monthly().unwrap();
    let result = purchases.purchase_package(monthly).await.unwrap();

    // Assert: 3 attempts total, purchase granted.
    assert!(result.customer_info.entitlements.is_active("pro"));
    assert_eq!(server.received_on("/v1/receipts").len(), 3);
}

#[tokio::test]
async fn a_third_429_exhausts_the_retry_budget() {
    // Arrange: more 429s than the 3-attempt schedule allows.
    let server = spawn_mock().await;
    server.rate_limit_receipts(3);
    let purchases = configure(&server, Some("retry-user"));

    let offerings = purchases.get_offerings().await.unwrap();
    let monthly = offerings.current().unwrap().monthly().unwrap();
    let error = purchases.purchase_package(monthly).await.unwrap_err();

    assert_eq!(error.code, ErrorCode::NetworkError);
    assert_eq!(server.received_on("/v1/receipts").len(), 3);
}

#[tokio::test]
async fn a_304_without_local_cache_is_retried_once_with_empty_etag() {
    // Arrange: the server incorrectly 304s a client that has no cache.
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("etag-miss"));
    server.force_304_next("/v1/subscribers/etag-miss/offerings");

    // Act: the client must recover by retrying with a cache-busting
    // empty ETag, mirroring ETagManager's single retry.
    let offerings = purchases.get_offerings().await.unwrap();

    // Assert
    assert!(offerings.current().is_some());
    let requests = server.received_on("/v1/subscribers/etag-miss/offerings");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].headers["x-revenuecat-etag"], "");
}

#[tokio::test]
async fn offerings_expose_the_dashboard_paywall() {
    // Arrange
    let server = spawn_mock().await;
    let purchases = configure(&server, Some("paywall-user"));

    // Act
    let offerings = purchases.get_offerings().await.unwrap();
    let current = offerings.current().expect("current offering");
    let paywall = current
        .paywall
        .as_ref()
        .expect("offering carries a paywall");

    // Assert: the v1 paywall config + localized copy round-trips.
    assert_eq!(paywall.template_name, "2");
    assert!(paywall.config.display_restore_purchases);
    assert!(paywall.config.packages.contains(&"$rc_monthly".to_owned()));
    let en = paywall.strings_for("en_US");
    assert_eq!(en.title.as_deref(), Some("Unlock Pro"));
    assert_eq!(en.features.len(), 3);
    assert_eq!(
        paywall
            .config
            .colors
            .light
            .call_to_action_background
            .as_deref(),
        Some("#e0554d")
    );

    // Every paywall package id resolves to a package in the offering.
    for id in &paywall.config.packages {
        assert!(
            current.package(id).is_some(),
            "paywall package {id} must resolve"
        );
    }
}
