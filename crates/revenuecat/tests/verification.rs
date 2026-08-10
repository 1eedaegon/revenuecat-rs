//! Trusted Entitlements end-to-end tests: the SDK verifies real Ed25519
//! signatures produced by the mock backend's test root -> intermediate ->
//! payload chain, across modes, tampering, ETag replay, and signed POSTs.
//! Plus web purchase redemption, which shares the signed-request machinery.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use revenuecat::{
    CacheFetchPolicy, Configuration, EntitlementVerificationMode, ErrorCode, Purchases,
    RedeemResult, VerificationResult, WebPurchaseRedemption, ROOT_PUBLIC_KEY_B64,
};
use revenuecat_mock::{test_root_public_key_b64, MockRevenueCat, MockServerHandle};

const TEST_API_KEY: &str = "test_verification_key";

fn configure_with_mode(
    server: &MockServerHandle,
    app_user_id: &str,
    mode: EntitlementVerificationMode,
    root_key: &str,
) -> Purchases {
    Purchases::configure(
        Configuration::builder(TEST_API_KEY)
            .proxy_url(&server.url)
            .app_user_id(app_user_id)
            .entitlement_verification_mode(mode)
            .verification_root_key(root_key)
            .http_timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
    )
    .unwrap()
}

fn informational(server: &MockServerHandle, app_user_id: &str) -> Purchases {
    configure_with_mode(
        server,
        app_user_id,
        EntitlementVerificationMode::Informational,
        &test_root_public_key_b64(),
    )
}

#[tokio::test]
async fn informational_mode_verifies_signed_responses() {
    // Arrange
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = informational(&server, "gon");

    // Act: customer info + a full purchase, both on verified endpoints.
    let info = purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
        .unwrap();
    let offerings = purchases.get_offerings().await.unwrap();
    let monthly = offerings.current().unwrap().monthly().unwrap();
    let result = purchases.purchase_package(monthly).await.unwrap();

    // Assert: Ed25519 chain verified end-to-end.
    assert_eq!(info.entitlements.verification, VerificationResult::Verified);
    assert_eq!(
        result.customer_info.entitlements.verification,
        VerificationResult::Verified
    );
    assert!(result.customer_info.entitlements.all["pro"]
        .verification
        .is_verified());
}

#[tokio::test]
async fn signed_posts_carry_nonce_and_post_params_hash() {
    // Arrange
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = informational(&server, "gon");

    // Act
    let offerings = purchases.get_offerings().await.unwrap();
    let monthly = offerings.current().unwrap().monthly().unwrap();
    purchases.purchase_package(monthly).await.unwrap();
    purchases.log_in("gon-pro").await.unwrap();

    // Assert: the receipt post was signed; the mock validates the hash
    // value server-side (a wrong hash would have failed the purchase).
    let receipt = &server.received_on("/v1/receipts")[0];
    assert!(!receipt.headers["x-nonce"].is_empty());
    assert!(receipt.headers["x-post-params-hash"].starts_with("app_user_id,fetch_token:sha256:"));

    let login = &server.received_on("/v1/subscribers/identify")[0];
    assert!(login.headers["x-post-params-hash"].starts_with("app_user_id,new_app_user_id:sha256:"));

    // GET offerings is verified but nonce-less, like the official SDKs.
    let offerings_request = &server.received_on("/v1/subscribers/gon/offerings")[0];
    assert!(!offerings_request.headers.contains_key("x-nonce"));
}

#[tokio::test]
async fn enforced_mode_turns_tampered_signatures_into_errors() {
    // Arrange: the mock corrupts every payload signature.
    let server = MockRevenueCat::with_default_catalog()
        .tamper_signatures()
        .spawn()
        .await
        .unwrap();
    let purchases = configure_with_mode(
        &server,
        "gon",
        EntitlementVerificationMode::Enforced,
        &test_root_public_key_b64(),
    );

    // Act
    let error = purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
        .unwrap_err();

    // Assert
    assert_eq!(error.code, ErrorCode::SignatureVerificationError);
}

#[tokio::test]
async fn informational_mode_marks_tampered_signatures_as_failed() {
    let server = MockRevenueCat::with_default_catalog()
        .tamper_signatures()
        .spawn()
        .await
        .unwrap();
    let purchases = informational(&server, "gon");

    let info = purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
        .unwrap();

    // Data still flows (informational), but flagged as FAILED.
    assert_eq!(info.entitlements.verification, VerificationResult::Failed);
}

#[tokio::test]
async fn wrong_root_key_fails_verification() {
    // Arrange: SDK trusts RevenueCat's production root, mock signs with the
    // test chain — the intermediate key must be rejected.
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = configure_with_mode(
        &server,
        "gon",
        EntitlementVerificationMode::Informational,
        ROOT_PUBLIC_KEY_B64,
    );

    let info = purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
        .unwrap();

    assert_eq!(info.entitlements.verification, VerificationResult::Failed);
}

#[tokio::test]
async fn disabled_mode_reports_not_requested() {
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder(TEST_API_KEY)
            .proxy_url(&server.url)
            .app_user_id("gon")
            .build()
            .unwrap(),
    )
    .unwrap();

    let info = purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
        .unwrap();

    assert_eq!(
        info.entitlements.verification,
        VerificationResult::NotRequested
    );
}

#[tokio::test]
async fn etag_304_replay_is_verified_via_fresh_signature() {
    // Arrange
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = informational(&server, "etag-user");

    // Act: the second fetch revalidates via ETag; the mock signs the 304
    // over `etag + empty body` and the replayed body must come back Verified.
    let first = purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
        .unwrap();
    let second = purchases
        .get_customer_info(CacheFetchPolicy::FetchCurrent)
        .await
        .unwrap();

    // Assert
    assert_eq!(
        first.entitlements.verification,
        VerificationResult::Verified
    );
    assert_eq!(
        second.entitlements.verification,
        VerificationResult::Verified
    );
    let requests = server.received_on("/v1/subscribers/etag-user");
    assert_eq!(requests.len(), 2);
    assert!(
        !requests[1].headers["x-revenuecat-etag"].is_empty(),
        "second request must revalidate via ETag"
    );
}

// ---------------------------------------------------------------------------
// Web purchase redemption
// ---------------------------------------------------------------------------

#[tokio::test]
async fn redeeming_a_valid_link_grants_the_purchase() {
    // Arrange
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = informational(&server, "gon");
    let redemption = Purchases::parse_web_purchase_redemption(
        "myapp://redeem_web_purchase?redemption_token=rdm_valid",
    )
    .unwrap();

    // Act
    let result = purchases.redeem_web_purchase(&redemption).await.unwrap();

    // Assert: the web purchase (annual -> pro) landed on this user, verified.
    let RedeemResult::Success { customer_info } = result else {
        panic!("expected success, got {result:?}");
    };
    assert!(customer_info.entitlements.is_active("pro"));
    assert!(customer_info.active_subscriptions().contains("annual"));
    assert_eq!(
        customer_info.entitlements.verification,
        VerificationResult::Verified
    );

    // And the wire body matched the official SDKs'.
    let request = &server.received_on("/v1/subscribers/redeem_purchase")[0];
    let body = request.body.as_ref().unwrap();
    assert_eq!(body["redemption_token"], "rdm_valid");
    assert_eq!(body["app_user_id"], "gon");
    assert!(!request.headers["x-nonce"].is_empty());
}

#[tokio::test]
async fn redemption_failures_map_to_typed_results() {
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = informational(&server, "gon");

    // Unknown token -> InvalidToken (7849).
    let invalid = WebPurchaseRedemption {
        redemption_token: "nope".into(),
    };
    assert!(matches!(
        purchases.redeem_web_purchase(&invalid).await.unwrap(),
        RedeemResult::InvalidToken
    ));

    // Expired token -> Expired with the server-obfuscated email (7853).
    let expired = WebPurchaseRedemption {
        redemption_token: "rdm_expired".into(),
    };
    let result = purchases.redeem_web_purchase(&expired).await.unwrap();
    let RedeemResult::Expired { obfuscated_email } = result else {
        panic!("expected expired, got {result:?}");
    };
    assert_eq!(obfuscated_email, "g***@e*****e.com");
}

#[tokio::test]
async fn redeeming_anothers_purchase_is_rejected() {
    // Arrange: user A redeems first.
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let user_a = informational(&server, "user-a");
    let token = WebPurchaseRedemption {
        redemption_token: "rdm_valid".into(),
    };
    assert!(user_a
        .redeem_web_purchase(&token)
        .await
        .unwrap()
        .is_success());

    // Act: user B tries the same link.
    let user_b = informational(&server, "user-b");
    let result = user_b.redeem_web_purchase(&token).await.unwrap();

    // Assert (7852).
    assert!(matches!(result, RedeemResult::PurchaseBelongsToOtherUser));
}
