//! Contract tests for the SDK ↔ native-store seam, using a fake
//! `StoreBilling` that behaves like the mobile shims (StoreKit 2 / Play
//! Billing): JWS-style purchase tokens the backend cannot parse as Test
//! Store tokens, and a recorded `finish_transaction` consume flag.
//!
//! These are the headless equivalent of the device E2E runs: they pin the
//! exact wire + finish-ordering behavior the Tauri plugin relies on.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use revenuecat::{
    Configuration, Error, ErrorCode, Price, ProductType, Purchases, Result, Store, StoreBilling,
    StoreProduct, StoreTransaction,
};
use revenuecat_mock::MockRevenueCat;

/// A JWS-shaped token, as StoreKit 2 hands over — decidedly not `test_…`.
const JWS_TOKEN_PREFIX: &str = "eyJhbGciOiJFUzI1NiJ9.native-";

#[derive(Debug, Clone, PartialEq)]
struct FinishCall {
    purchase_token: String,
    should_consume: bool,
}

/// Fake native store: serves a catalog like the mobile shims would and
/// records every `finish_transaction` call.
struct FakeNativeBilling {
    catalog: Vec<StoreProduct>,
    owned: Vec<StoreTransaction>,
    finish_calls: Arc<Mutex<Vec<FinishCall>>>,
    purchase_counter: Mutex<u64>,
}

impl FakeNativeBilling {
    fn with_catalog(catalog: Vec<StoreProduct>) -> Self {
        Self {
            catalog,
            owned: Vec::new(),
            finish_calls: Arc::new(Mutex::new(Vec::new())),
            purchase_counter: Mutex::new(0),
        }
    }

    fn owning(mut self, transactions: Vec<StoreTransaction>) -> Self {
        self.owned = transactions;
        self
    }

    fn finish_calls_handle(&self) -> Arc<Mutex<Vec<FinishCall>>> {
        Arc::clone(&self.finish_calls)
    }
}

#[async_trait]
impl StoreBilling for FakeNativeBilling {
    async fn query_products(&self, product_ids: &[String]) -> Result<Vec<StoreProduct>> {
        Ok(self
            .catalog
            .iter()
            .filter(|p| product_ids.contains(&p.identifier))
            .cloned()
            .collect())
    }

    async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction> {
        let mut counter = self.purchase_counter.lock().unwrap();
        *counter += 1;
        Ok(StoreTransaction {
            purchase_token: format!("{JWS_TOKEN_PREFIX}{}-{}", product.identifier, *counter),
            product_ids: vec![product.identifier.clone()],
            purchase_date: Utc::now(),
            transaction_id: Some(format!("2000000{counter}")),
            store: Store::AppStore,
            price: Some(product.price.clone()),
        })
    }

    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
        Ok(self.owned.clone())
    }

    async fn finish_transaction(
        &self,
        transaction: &StoreTransaction,
        should_consume: bool,
    ) -> Result<()> {
        self.finish_calls.lock().unwrap().push(FinishCall {
            purchase_token: transaction.purchase_token.clone(),
            should_consume,
        });
        Ok(())
    }
}

fn native_catalog() -> Vec<StoreProduct> {
    vec![
        StoreProduct {
            identifier: "monthly".into(),
            product_type: ProductType::Subscription,
            title: "Monthly Pro (StoreKit)".into(),
            description: Some("From the native store".into()),
            price: Price {
                amount_micros: 2_990_000,
                currency: "USD".into(),
            },
            subscription_period: Some("P1M".into()),
            trial: None,
            intro_price: None,
        },
        StoreProduct {
            identifier: "annual".into(),
            product_type: ProductType::Subscription,
            title: "Annual Pro (StoreKit)".into(),
            description: None,
            price: Price {
                amount_micros: 29_990_000,
                currency: "USD".into(),
            },
            subscription_period: Some("P1Y".into()),
            trial: None,
            intro_price: None,
        },
        StoreProduct {
            identifier: "coins_100".into(),
            product_type: ProductType::Consumable,
            title: "100 Coins (StoreKit)".into(),
            description: None,
            price: Price {
                amount_micros: 990_000,
                currency: "USD".into(),
            },
            subscription_period: None,
            trial: None,
            intro_price: None,
        },
    ]
}

async fn purchases_against(
    mock_url: &str,
    billing: FakeNativeBilling,
) -> (Purchases, Arc<Mutex<Vec<FinishCall>>>) {
    let finish_calls = billing.finish_calls_handle();
    let purchases = Purchases::configure(
        Configuration::builder("appl_fake_key")
            .app_user_id("native-user")
            .proxy_url(mock_url)
            .store_billing(Arc::new(billing))
            .build()
            .unwrap(),
    )
    .unwrap();
    (purchases, finish_calls)
}

#[tokio::test]
async fn offerings_join_backend_packages_with_native_products() {
    // Arrange
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let (purchases, _) =
        purchases_against(&mock.url, FakeNativeBilling::with_catalog(native_catalog())).await;

    // Act
    let offerings = purchases.get_offerings().await.unwrap();

    // Assert: packages resolved against the NATIVE catalog (titles/prices
    // prove products came from StoreBilling, not the mock's product feed).
    let current = offerings.current().unwrap();
    let monthly = current.monthly().unwrap();
    assert_eq!(monthly.store_product.title, "Monthly Pro (StoreKit)");
    assert_eq!(monthly.store_product.price.amount_micros, 2_990_000);
    assert_eq!(current.packages.len(), 3);
}

#[tokio::test]
async fn purchase_with_store_token_succeeds_when_mock_accepts_them() {
    // Arrange
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let (purchases, _) =
        purchases_against(&mock.url, FakeNativeBilling::with_catalog(native_catalog())).await;
    let offerings = purchases.get_offerings().await.unwrap();
    let monthly = offerings.current().unwrap().monthly().unwrap().clone();

    // Act
    let result = purchases.purchase_package(&monthly).await.unwrap();

    // Assert: JWS-style token was accepted and the entitlement is active.
    assert!(result
        .transaction
        .purchase_token
        .starts_with(JWS_TOKEN_PREFIX));
    assert!(result.customer_info.entitlements.is_active("pro"));

    // And the receipt hit the wire with the native token as fetch_token.
    let receipts = mock.received_on("/v1/receipts");
    assert_eq!(receipts.len(), 1);
    let body = receipts[0].body.as_ref().unwrap();
    assert_eq!(
        body["fetch_token"].as_str().unwrap(),
        result.transaction.purchase_token
    );
    assert_eq!(body["presented_offering_identifier"], "default");
}

#[tokio::test]
async fn mock_rejects_store_tokens_by_default_as_invalid_receipt() {
    // Arrange: default-strict mock (mirrors the real backend refusing tokens
    // it cannot associate with a store app).
    let mock = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let (purchases, finish_calls) =
        purchases_against(&mock.url, FakeNativeBilling::with_catalog(native_catalog())).await;
    let offerings = purchases.get_offerings().await.unwrap();
    let monthly = offerings.current().unwrap().monthly().unwrap().clone();

    // Act
    let error = purchases.purchase_package(&monthly).await.unwrap_err();

    // Assert: typed backend error, and the transaction was NOT finished —
    // an unfinished transaction is redelivered by the store, so the purchase
    // isn't lost.
    assert_eq!(error.code, ErrorCode::InvalidReceiptError);
    assert!(finish_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn subscription_purchase_finishes_without_consuming() {
    // Arrange
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let (purchases, finish_calls) =
        purchases_against(&mock.url, FakeNativeBilling::with_catalog(native_catalog())).await;
    let offerings = purchases.get_offerings().await.unwrap();
    let monthly = offerings.current().unwrap().monthly().unwrap().clone();

    // Act
    purchases.purchase_package(&monthly).await.unwrap();

    // Assert: exactly one finish, and a SUBSCRIPTION must be acknowledged,
    // never consumed (consuming a subscription token is a Play Billing
    // developer error).
    let calls = finish_calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(
        !calls[0].should_consume,
        "subscriptions must be finished with should_consume=false, got consume=true"
    );
}

#[tokio::test]
async fn consumable_purchase_finishes_with_consume() {
    // Arrange
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let (purchases, finish_calls) =
        purchases_against(&mock.url, FakeNativeBilling::with_catalog(native_catalog())).await;
    let offerings = purchases.get_offerings().await.unwrap();
    let coins = offerings
        .current()
        .unwrap()
        .package("coins")
        .unwrap()
        .clone();

    // Act
    purchases.purchase_package(&coins).await.unwrap();

    // Assert: consumables ARE consumed so they can be repurchased.
    let calls = finish_calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].should_consume,
        "consumables must be finished with should_consume=true"
    );
}

#[tokio::test]
async fn store_token_reused_by_other_user_maps_to_receipt_already_in_use() {
    // Arrange: user A redeems a native token, then user B posts the same one.
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let token = format!("{JWS_TOKEN_PREFIX}monthly-shared");
    let shared_transaction = StoreTransaction {
        purchase_token: token.clone(),
        product_ids: vec!["monthly".into()],
        purchase_date: Utc::now(),
        transaction_id: Some("20000001".into()),
        store: Store::AppStore,
        price: None,
    };

    let (user_a, _) = purchases_against(
        &mock.url,
        FakeNativeBilling::with_catalog(native_catalog()).owning(vec![shared_transaction.clone()]),
    )
    .await;
    user_a.restore_purchases().await.unwrap();

    let user_b = Purchases::configure(
        Configuration::builder("appl_fake_key")
            .app_user_id("other-user")
            .proxy_url(&mock.url)
            .store_billing(Arc::new(
                FakeNativeBilling::with_catalog(native_catalog()).owning(vec![shared_transaction]),
            ))
            .build()
            .unwrap(),
    )
    .unwrap();

    // Act
    let error = user_b.restore_purchases().await.unwrap_err();

    // Assert
    assert_eq!(error.code, ErrorCode::ReceiptAlreadyInUseError);
}

#[tokio::test]
async fn restore_posts_each_owned_transaction_as_restore_and_finishes_without_consume() {
    // Arrange
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let owned = vec![
        StoreTransaction {
            purchase_token: format!("{JWS_TOKEN_PREFIX}monthly-r1"),
            product_ids: vec!["monthly".into()],
            purchase_date: Utc::now(),
            transaction_id: Some("20000010".into()),
            store: Store::AppStore,
            price: None,
        },
        StoreTransaction {
            purchase_token: format!("{JWS_TOKEN_PREFIX}annual-r2"),
            product_ids: vec!["annual".into()],
            purchase_date: Utc::now(),
            transaction_id: Some("20000011".into()),
            store: Store::AppStore,
            price: None,
        },
    ];
    let (purchases, finish_calls) = purchases_against(
        &mock.url,
        FakeNativeBilling::with_catalog(native_catalog()).owning(owned),
    )
    .await;

    // Act
    let info = purchases.restore_purchases().await.unwrap();

    // Assert: both restores hit the wire flagged as restores…
    let receipts = mock.received_on("/v1/receipts");
    assert_eq!(receipts.len(), 2);
    for receipt in &receipts {
        let body = receipt.body.as_ref().unwrap();
        assert_eq!(body["is_restore"], true);
        assert_eq!(body["initiation_source"], "restore");
    }
    // …the entitlement is active, and restored transactions are finished
    // without consuming.
    assert!(info.entitlements.is_active("pro"));
    let calls = finish_calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|c| !c.should_consume));
}

/// Guard: an error mid-way (`query_products` fails on the native side) must
/// surface as the billing error, not a panic or a silent empty offering.
#[tokio::test]
async fn native_query_products_error_propagates_from_get_offerings() {
    struct BrokenBilling;

    #[async_trait]
    impl StoreBilling for BrokenBilling {
        async fn query_products(&self, _: &[String]) -> Result<Vec<StoreProduct>> {
            Err(Error::new(
                ErrorCode::StoreProblemError,
                "Billing is not available on this device: BILLING_UNAVAILABLE",
            ))
        }
        async fn purchase(&self, _: &StoreProduct) -> Result<StoreTransaction> {
            panic!("purchase must not be reached")
        }
        async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
            Ok(Vec::new())
        }
        async fn finish_transaction(&self, _: &StoreTransaction, _: bool) -> Result<()> {
            Ok(())
        }
    }

    let mock = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("goog_fake_key")
            .app_user_id("android-user")
            .proxy_url(&mock.url)
            .store_billing(Arc::new(BrokenBilling))
            .build()
            .unwrap(),
    )
    .unwrap();

    let error = purchases.get_offerings().await.unwrap_err();

    assert_eq!(error.code, ErrorCode::StoreProblemError);
    assert!(error.to_string().contains("BILLING_UNAVAILABLE"));
}
