//! Contract tests for the SDK ↔ native-store seam, using a fake
//! `StoreBilling` that behaves like the mobile shims (StoreKit 2 / Play
//! Billing): JWS-style purchase tokens the backend cannot parse as Test
//! Store tokens, and a recorded `finish_transaction` consume flag.
//!
//! These are the headless equivalent of the device E2E runs: they pin the
//! exact wire + finish-ordering behavior the Tauri plugin relies on.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use revenuecat::{
    CacheFetchPolicy, Configuration, Error, ErrorCode, Price, ProductType, Purchases, Result,
    Store, StoreBilling, StoreProduct, StorePurchase, StorePurchaseOption, StoreReceiptContext,
    StoreTransaction, TargetingResponse,
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
    owned_with_context: Option<Vec<StorePurchase>>,
    finish_calls: Arc<Mutex<Vec<FinishCall>>>,
    purchase_counter: Mutex<u64>,
}

struct StatefulPendingBilling {
    catalog: Vec<StoreProduct>,
    pending: Arc<Mutex<Vec<StoreTransaction>>>,
    finish_calls: Arc<Mutex<Vec<FinishCall>>>,
    purchase_calls: Arc<AtomicUsize>,
}

impl FakeNativeBilling {
    fn with_catalog(catalog: Vec<StoreProduct>) -> Self {
        Self {
            catalog,
            owned: Vec::new(),
            owned_with_context: None,
            finish_calls: Arc::new(Mutex::new(Vec::new())),
            purchase_counter: Mutex::new(0),
        }
    }

    fn owning(mut self, transactions: Vec<StoreTransaction>) -> Self {
        self.owned = transactions;
        self
    }

    fn owning_with_context(mut self, purchases: Vec<StorePurchase>) -> Self {
        self.owned_with_context = Some(purchases);
        self
    }

    fn finish_calls_handle(&self) -> Arc<Mutex<Vec<FinishCall>>> {
        Arc::clone(&self.finish_calls)
    }
}

#[async_trait]
impl StoreBilling for StatefulPendingBilling {
    async fn query_products(&self, product_ids: &[String]) -> Result<Vec<StoreProduct>> {
        Ok(self
            .catalog
            .iter()
            .filter(|product| product_ids.contains(&product.identifier))
            .cloned()
            .collect())
    }

    async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction> {
        let purchase_number = self.purchase_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let transaction = StoreTransaction {
            purchase_token: format!(
                "{JWS_TOKEN_PREFIX}{}-stateful-{purchase_number}",
                product.identifier
            ),
            product_ids: vec![product.identifier.clone()],
            purchase_date: Utc::now(),
            transaction_id: Some(format!("2000002{purchase_number}")),
            store: Store::AppStore,
            price: Some(product.price.clone()),
        };
        self.pending.lock().unwrap().push(transaction.clone());
        Ok(transaction)
    }

    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
        Ok(self.pending.lock().unwrap().clone())
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
        self.pending
            .lock()
            .unwrap()
            .retain(|pending| pending.purchase_token != transaction.purchase_token);
        Ok(())
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

    async fn query_purchases_with_context(&self) -> Result<Vec<StorePurchase>> {
        Ok(self.owned_with_context.clone().unwrap_or_else(|| {
            self.owned
                .iter()
                .cloned()
                .map(StorePurchase::from_transaction)
                .collect()
        }))
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
async fn user_aware_purchase_default_keeps_existing_billing_implementations_working() {
    let billing = FakeNativeBilling::with_catalog(native_catalog());
    let product = native_catalog().remove(0);

    let purchase = billing
        .purchase_with_option_for_user(&product, None, "current-user")
        .await
        .unwrap();

    assert_eq!(purchase.transaction.product_ids, ["monthly"]);
    assert_eq!(purchase.platform_product_ids[0].product_id(), "monthly");
}

struct BlockingPurchaseBilling {
    purchase_started: Arc<tokio::sync::Notify>,
    release_purchase: Arc<tokio::sync::Notify>,
}

struct BlockingProductQueryBilling {
    catalog: Vec<StoreProduct>,
    query_started: Arc<tokio::sync::Notify>,
    release_query: Arc<tokio::sync::Notify>,
}

struct PurchaseErrorAfterTransactionBilling {
    transaction_available: AtomicBool,
}

struct FinishErrorOnceBilling {
    transaction_available: Arc<AtomicBool>,
    finish_attempts: Arc<Mutex<u32>>,
}

struct PurchaseCallTrackingBilling {
    purchase_called: Arc<AtomicBool>,
}

struct UserAwarePurchaseBilling {
    observed_app_user_ids: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl StoreBilling for UserAwarePurchaseBilling {
    async fn query_products(&self, _: &[String]) -> Result<Vec<StoreProduct>> {
        Ok(Vec::new())
    }

    async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction> {
        Ok(StoreTransaction {
            purchase_token: format!("{JWS_TOKEN_PREFIX}{}-user-aware", product.identifier),
            product_ids: vec![product.identifier.clone()],
            purchase_date: Utc::now(),
            transaction_id: Some("200000100".into()),
            store: Store::AppStore,
            price: Some(product.price.clone()),
        })
    }

    async fn purchase_with_option_for_user(
        &self,
        product: &StoreProduct,
        _: Option<&StorePurchaseOption>,
        app_user_id: &str,
    ) -> Result<StorePurchase> {
        self.observed_app_user_ids
            .lock()
            .unwrap()
            .push(app_user_id.to_owned());
        self.purchase(product)
            .await
            .map(StorePurchase::from_transaction)
    }

    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
        Ok(Vec::new())
    }

    async fn finish_transaction(&self, _: &StoreTransaction, _: bool) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl StoreBilling for BlockingPurchaseBilling {
    async fn query_products(&self, _: &[String]) -> Result<Vec<StoreProduct>> {
        Ok(Vec::new())
    }

    async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction> {
        self.purchase_started.notify_one();
        self.release_purchase.notified().await;
        Ok(StoreTransaction {
            purchase_token: format!("{JWS_TOKEN_PREFIX}{}-identity", product.identifier),
            product_ids: vec![product.identifier.clone()],
            purchase_date: Utc::now(),
            transaction_id: Some("200000099".into()),
            store: Store::AppStore,
            price: Some(product.price.clone()),
        })
    }

    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
        Ok(Vec::new())
    }

    async fn finish_transaction(&self, _: &StoreTransaction, _: bool) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl StoreBilling for BlockingProductQueryBilling {
    async fn query_products(&self, product_ids: &[String]) -> Result<Vec<StoreProduct>> {
        self.query_started.notify_one();
        self.release_query.notified().await;
        Ok(self
            .catalog
            .iter()
            .filter(|product| product_ids.contains(&product.identifier))
            .cloned()
            .collect())
    }

    async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction> {
        Ok(StoreTransaction {
            purchase_token: format!("{JWS_TOKEN_PREFIX}{}-query-race", product.identifier),
            product_ids: vec![product.identifier.clone()],
            purchase_date: Utc::now(),
            transaction_id: Some("200000103".into()),
            store: Store::AppStore,
            price: Some(product.price.clone()),
        })
    }

    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
        Ok(Vec::new())
    }

    async fn finish_transaction(&self, _: &StoreTransaction, _: bool) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl StoreBilling for PurchaseErrorAfterTransactionBilling {
    async fn query_products(&self, _: &[String]) -> Result<Vec<StoreProduct>> {
        Ok(Vec::new())
    }

    async fn purchase(&self, _: &StoreProduct) -> Result<StoreTransaction> {
        self.transaction_available.store(true, Ordering::Release);
        Err(Error::new(
            ErrorCode::StoreProblemError,
            "native response failed after the store created a transaction",
        ))
    }

    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
        if !self.transaction_available.load(Ordering::Acquire) {
            return Ok(Vec::new());
        }
        let product = native_catalog().remove(0);
        Ok(vec![StoreTransaction {
            purchase_token: format!("{JWS_TOKEN_PREFIX}monthly-response-failure"),
            product_ids: vec![product.identifier],
            purchase_date: Utc::now(),
            transaction_id: Some("200000101".into()),
            store: Store::AppStore,
            price: Some(product.price),
        }])
    }

    async fn finish_transaction(&self, _: &StoreTransaction, _: bool) -> Result<()> {
        self.transaction_available.store(false, Ordering::Release);
        Ok(())
    }
}

#[async_trait]
impl StoreBilling for FinishErrorOnceBilling {
    async fn query_products(&self, _: &[String]) -> Result<Vec<StoreProduct>> {
        Ok(Vec::new())
    }

    async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction> {
        self.transaction_available.store(true, Ordering::Release);
        Ok(StoreTransaction {
            purchase_token: format!("{JWS_TOKEN_PREFIX}{}-finish-failure", product.identifier),
            product_ids: vec![product.identifier.clone()],
            purchase_date: Utc::now(),
            transaction_id: Some("200000104".into()),
            store: Store::AppStore,
            price: Some(product.price.clone()),
        })
    }

    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
        if !self.transaction_available.load(Ordering::Acquire) {
            return Ok(Vec::new());
        }
        let product = native_catalog().remove(0);
        Ok(vec![StoreTransaction {
            purchase_token: format!("{JWS_TOKEN_PREFIX}{}-finish-failure", product.identifier),
            product_ids: vec![product.identifier],
            purchase_date: Utc::now(),
            transaction_id: Some("200000104".into()),
            store: Store::AppStore,
            price: Some(product.price),
        }])
    }

    async fn finish_transaction(&self, _: &StoreTransaction, _: bool) -> Result<()> {
        let attempt = {
            let mut attempts = self.finish_attempts.lock().unwrap();
            *attempts += 1;
            *attempts
        };
        if attempt == 1 {
            return Err(Error::new(
                ErrorCode::StoreProblemError,
                "temporary native finish failure",
            ));
        }
        self.transaction_available.store(false, Ordering::Release);
        Ok(())
    }
}

#[async_trait]
impl StoreBilling for PurchaseCallTrackingBilling {
    async fn query_products(&self, _: &[String]) -> Result<Vec<StoreProduct>> {
        Ok(Vec::new())
    }

    async fn purchase(&self, product: &StoreProduct) -> Result<StoreTransaction> {
        self.purchase_called.store(true, Ordering::Release);
        Ok(StoreTransaction {
            purchase_token: format!("{JWS_TOKEN_PREFIX}{}-invalid-input", product.identifier),
            product_ids: vec![product.identifier.clone()],
            purchase_date: Utc::now(),
            transaction_id: Some("200000102".into()),
            store: Store::AppStore,
            price: Some(product.price.clone()),
        })
    }

    async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
        Ok(Vec::new())
    }

    async fn finish_transaction(&self, _: &StoreTransaction, _: bool) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn identity_change_waits_for_the_entire_native_purchase_flow() {
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let purchase_started = Arc::new(tokio::sync::Notify::new());
    let release_purchase = Arc::new(tokio::sync::Notify::new());
    let purchases = Purchases::configure(
        Configuration::builder("appl_fake_key")
            .app_user_id("native-user")
            .proxy_url(&mock.url)
            .store_billing(Arc::new(BlockingPurchaseBilling {
                purchase_started: Arc::clone(&purchase_started),
                release_purchase: Arc::clone(&release_purchase),
            }))
            .build()
            .unwrap(),
    )
    .unwrap();
    let product = native_catalog().remove(0);

    let purchase_sdk = purchases.clone();
    let purchase = tokio::spawn(async move { purchase_sdk.purchase_product(&product).await });
    purchase_started.notified().await;

    let logout_sdk = purchases.clone();
    let mut logout = tokio::spawn(async move { logout_sdk.log_out().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut logout)
            .await
            .is_err(),
        "logOut must wait while the native purchase sheet and receipt post are active"
    );

    release_purchase.notify_one();
    purchase.await.unwrap().unwrap();
    logout.await.unwrap().unwrap();

    let receipts = mock.received_on("/v1/receipts");
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts[0].body.as_ref().unwrap()["app_user_id"],
        "native-user"
    );
}

#[tokio::test]
async fn native_purchase_response_failure_requires_recovery_before_identity_change() {
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("appl_fake_key")
            .app_user_id("native-user")
            .proxy_url(&mock.url)
            .store_billing(Arc::new(PurchaseErrorAfterTransactionBilling {
                transaction_available: AtomicBool::new(false),
            }))
            .build()
            .unwrap(),
    )
    .unwrap();
    let product = native_catalog().remove(0);

    let purchase_error = purchases.purchase_product(&product).await.unwrap_err();
    assert_eq!(purchase_error.code, ErrorCode::StoreProblemError);

    let retry_error = purchases.purchase_product(&product).await.unwrap_err();
    assert_eq!(retry_error.code, ErrorCode::OperationAlreadyInProgressError);
    let login_error = purchases.log_in("other-user").await.unwrap_err();
    assert_eq!(login_error.code, ErrorCode::OperationAlreadyInProgressError);
    let logout_error = purchases.log_out().await.unwrap_err();
    assert_eq!(
        logout_error.code,
        ErrorCode::OperationAlreadyInProgressError
    );

    purchases.sync_pending_purchases().await.unwrap();
    purchases.log_in("other-user").await.unwrap();
    assert_eq!(purchases.app_user_id(), "other-user");
}

#[tokio::test]
async fn native_finish_failure_requires_queue_recovery_before_identity_change() {
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let transaction_available = Arc::new(AtomicBool::new(false));
    let finish_attempts = Arc::new(Mutex::new(0));
    let purchases = Purchases::configure(
        Configuration::builder("appl_fake_key")
            .app_user_id("native-user")
            .proxy_url(&mock.url)
            .store_billing(Arc::new(FinishErrorOnceBilling {
                transaction_available: Arc::clone(&transaction_available),
                finish_attempts: Arc::clone(&finish_attempts),
            }))
            .build()
            .unwrap(),
    )
    .unwrap();
    let product = native_catalog().remove(0);

    let finish_error = purchases.purchase_product(&product).await.unwrap_err();
    assert_eq!(finish_error.code, ErrorCode::StoreProblemError);
    assert_eq!(*finish_attempts.lock().unwrap(), 1);
    assert_eq!(mock.received_on("/v1/receipts").len(), 1);

    let purchase_error = purchases.purchase_product(&product).await.unwrap_err();
    assert_eq!(
        purchase_error.code,
        ErrorCode::OperationAlreadyInProgressError
    );
    let login_error = purchases.log_in("other-user").await.unwrap_err();
    assert_eq!(login_error.code, ErrorCode::OperationAlreadyInProgressError);

    purchases.sync_pending_purchases().await.unwrap();
    assert_eq!(*finish_attempts.lock().unwrap(), 2);
    assert!(!transaction_available.load(Ordering::Acquire));
    assert_eq!(mock.received_on("/v1/receipts").len(), 2);

    purchases.log_in("other-user").await.unwrap();
    assert_eq!(purchases.app_user_id(), "other-user");
}

#[tokio::test]
async fn invalid_manual_store_products_are_rejected_before_native_purchase() {
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let purchase_called = Arc::new(AtomicBool::new(false));
    let purchases = Purchases::configure(
        Configuration::builder("appl_fake_key")
            .app_user_id("native-user")
            .proxy_url(&mock.url)
            .store_billing(Arc::new(PurchaseCallTrackingBilling {
                purchase_called: Arc::clone(&purchase_called),
            }))
            .build()
            .unwrap(),
    )
    .unwrap();
    let product = native_catalog().remove(0);
    let invalid_products = [
        StoreProduct {
            identifier: " ".into(),
            ..product.clone()
        },
        StoreProduct {
            price: Price {
                amount_micros: -1,
                ..product.price.clone()
            },
            ..product.clone()
        },
        StoreProduct {
            price: Price {
                currency: "\t".into(),
                ..product.price.clone()
            },
            ..product.clone()
        },
        StoreProduct {
            subscription_period: Some("".into()),
            ..product
        },
    ];

    for product in invalid_products {
        let error = purchases.purchase_product(&product).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::ConfigurationError);
    }

    assert!(!purchase_called.load(Ordering::Acquire));
    assert!(mock.received_on("/v1/receipts").is_empty());
}

#[tokio::test]
async fn purchase_passes_the_current_app_user_id_to_native_billing() {
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let app_user_id = "123e4567-e89b-12d3-a456-426614174000";
    let observed_app_user_ids = Arc::new(Mutex::new(Vec::new()));
    let purchases = Purchases::configure(
        Configuration::builder("appl_fake_key")
            .app_user_id(app_user_id)
            .proxy_url(&mock.url)
            .store_billing(Arc::new(UserAwarePurchaseBilling {
                observed_app_user_ids: Arc::clone(&observed_app_user_ids),
            }))
            .build()
            .unwrap(),
    )
    .unwrap();
    let product = native_catalog().remove(0);

    purchases.purchase_product(&product).await.unwrap();

    assert_eq!(
        observed_app_user_ids.lock().unwrap().as_slice(),
        [app_user_id]
    );
}

#[tokio::test]
async fn concurrent_explicit_purchase_is_rejected_without_opening_a_second_store_flow() {
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let purchase_started = Arc::new(tokio::sync::Notify::new());
    let release_purchase = Arc::new(tokio::sync::Notify::new());
    let purchases = Purchases::configure(
        Configuration::builder("appl_fake_key")
            .app_user_id("native-user")
            .proxy_url(&mock.url)
            .store_billing(Arc::new(BlockingPurchaseBilling {
                purchase_started: Arc::clone(&purchase_started),
                release_purchase: Arc::clone(&release_purchase),
            }))
            .build()
            .unwrap(),
    )
    .unwrap();
    let product = native_catalog().remove(2);

    let first_sdk = purchases.clone();
    let first_product = product.clone();
    let first = tokio::spawn(async move { first_sdk.purchase_product(&first_product).await });
    purchase_started.notified().await;

    let second = tokio::time::timeout(
        Duration::from_millis(50),
        purchases.purchase_product(&product),
    )
    .await;
    release_purchase.notify_one();
    first.await.unwrap().unwrap();

    let error = second
        .expect("a concurrent purchase must be rejected immediately")
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::OperationAlreadyInProgressError);
    assert_eq!(mock.received_on("/v1/receipts").len(), 1);
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
async fn offerings_loaded_for_a_previous_identity_are_not_returned() {
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let query_started = Arc::new(tokio::sync::Notify::new());
    let release_query = Arc::new(tokio::sync::Notify::new());
    let purchases = Purchases::configure(
        Configuration::builder("appl_fake_key")
            .app_user_id("user-a")
            .proxy_url(&mock.url)
            .store_billing(Arc::new(BlockingProductQueryBilling {
                catalog: native_catalog(),
                query_started: Arc::clone(&query_started),
                release_query: Arc::clone(&release_query),
            }))
            .build()
            .unwrap(),
    )
    .unwrap();

    let loading_purchases = purchases.clone();
    let loading = tokio::spawn(async move { loading_purchases.get_offerings().await });
    query_started.notified().await;
    purchases.log_in("user-b").await.unwrap();
    release_query.notify_one();

    let error = loading.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::OperationAlreadyInProgressError);
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
    let pending = Arc::new(Mutex::new(Vec::new()));
    let finish_calls = Arc::new(Mutex::new(Vec::new()));
    let purchase_calls = Arc::new(AtomicUsize::new(0));
    let purchases = Purchases::configure(
        Configuration::builder("appl_fake_key")
            .app_user_id("native-user")
            .proxy_url(&mock.url)
            .store_billing(Arc::new(StatefulPendingBilling {
                catalog: native_catalog(),
                pending: Arc::clone(&pending),
                finish_calls: Arc::clone(&finish_calls),
                purchase_calls: Arc::clone(&purchase_calls),
            }))
            .build()
            .unwrap(),
    )
    .unwrap();
    let offerings = purchases.get_offerings().await.unwrap();
    let monthly = offerings.current().unwrap().monthly().unwrap().clone();

    // Act
    let error = purchases.purchase_package(&monthly).await.unwrap_err();

    // Assert: typed backend error, and the transaction was NOT finished —
    // an unfinished transaction is redelivered by the store, so the purchase
    // isn't lost.
    assert_eq!(error.code, ErrorCode::InvalidReceiptError);
    assert!(finish_calls.lock().unwrap().is_empty());
    let first_receipts = mock.received_on("/v1/receipts");
    assert_eq!(first_receipts.len(), 1);
    let pending_token = first_receipts[0].body.as_ref().unwrap()["fetch_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(purchase_calls.load(Ordering::SeqCst), 1);
    assert_eq!(pending.lock().unwrap().len(), 1);
    assert_eq!(pending.lock().unwrap()[0].purchase_token, pending_token);

    // A native transaction now exists even though posting failed. Do not
    // reopen store UI until an explicit queue reconciliation has run.
    let retry_error = purchases.purchase_package(&monthly).await.unwrap_err();
    assert_eq!(retry_error.code, ErrorCode::OperationAlreadyInProgressError);
    assert_eq!(mock.received_on("/v1/receipts").len(), 1);

    let sync_error = purchases.sync_pending_purchases().await.unwrap_err();
    assert_eq!(sync_error.code, ErrorCode::InvalidReceiptError);
    let receipts = mock.received_on("/v1/receipts");
    assert_eq!(receipts.len(), 2);
    assert!(receipts.iter().all(|receipt| {
        receipt.body.as_ref().unwrap()["fetch_token"] == pending_token.as_str()
    }));
    assert!(finish_calls.lock().unwrap().is_empty());
    assert_eq!(purchase_calls.load(Ordering::SeqCst), 1);
    assert_eq!(pending.lock().unwrap().len(), 1);

    let retry_error = purchases.purchase_package(&monthly).await.unwrap_err();
    assert_eq!(retry_error.code, ErrorCode::OperationAlreadyInProgressError);
    let login_error = purchases.log_in("other-user").await.unwrap_err();
    assert_eq!(login_error.code, ErrorCode::OperationAlreadyInProgressError);
    let logout_error = purchases.log_out().await.unwrap_err();
    assert_eq!(
        logout_error.code,
        ErrorCode::OperationAlreadyInProgressError
    );
}

#[tokio::test]
async fn failed_queue_sync_keeps_the_receipt_bound_to_the_original_identity() {
    let mock = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let owned = StoreTransaction {
        purchase_token: format!("{JWS_TOKEN_PREFIX}monthly-failed-sync"),
        product_ids: vec!["monthly".into()],
        purchase_date: Utc::now(),
        transaction_id: Some("20000016".into()),
        store: Store::AppStore,
        price: None,
    };
    let (purchases, _) = purchases_against(
        &mock.url,
        FakeNativeBilling::with_catalog(native_catalog()).owning(vec![owned]),
    )
    .await;

    let sync_error = purchases.sync_purchases().await.unwrap_err();
    assert_eq!(sync_error.code, ErrorCode::InvalidReceiptError);

    let login_error = purchases.log_in("other-user").await.unwrap_err();
    assert_eq!(login_error.code, ErrorCode::OperationAlreadyInProgressError);
    let logout_error = purchases.log_out().await.unwrap_err();
    assert_eq!(
        logout_error.code,
        ErrorCode::OperationAlreadyInProgressError
    );
}

#[tokio::test]
async fn partially_failed_queue_sync_keeps_the_recovery_barrier_armed() {
    let mock = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let owned = vec![
        StoreTransaction {
            purchase_token: "test_partial-queue-success".into(),
            product_ids: vec!["monthly".into()],
            purchase_date: Utc::now(),
            transaction_id: Some("20000017".into()),
            store: Store::AppStore,
            price: None,
        },
        StoreTransaction {
            purchase_token: format!("{JWS_TOKEN_PREFIX}annual-partial-queue-failure"),
            product_ids: vec!["annual".into()],
            purchase_date: Utc::now(),
            transaction_id: Some("20000018".into()),
            store: Store::AppStore,
            price: None,
        },
    ];
    let (purchases, finish_calls) = purchases_against(
        &mock.url,
        FakeNativeBilling::with_catalog(native_catalog()).owning(owned),
    )
    .await;
    let notifications = Arc::new(AtomicUsize::new(0));
    let listener_notifications = Arc::clone(&notifications);
    purchases.set_customer_info_listener(move |info| {
        assert!(info.entitlements.is_active("pro"));
        listener_notifications.fetch_add(1, Ordering::SeqCst);
    });

    let sync_error = purchases.sync_pending_purchases().await.unwrap_err();
    assert_eq!(sync_error.code, ErrorCode::InvalidReceiptError);
    assert_eq!(mock.received_on("/v1/receipts").len(), 2);
    assert_eq!(
        finish_calls.lock().unwrap().as_slice(),
        [FinishCall {
            purchase_token: "test_partial-queue-success".into(),
            should_consume: false,
        }]
    );
    let cached = purchases
        .get_customer_info(CacheFetchPolicy::CacheOnly)
        .await
        .unwrap();
    assert!(cached.entitlements.is_active("pro"));
    assert_eq!(notifications.load(Ordering::SeqCst), 1);

    let login_error = purchases.log_in("other-user").await.unwrap_err();
    assert_eq!(login_error.code, ErrorCode::OperationAlreadyInProgressError);
    let logout_error = purchases.log_out().await.unwrap_err();
    assert_eq!(
        logout_error.code,
        ErrorCode::OperationAlreadyInProgressError
    );
}

#[tokio::test]
async fn failed_queue_query_keeps_future_receipts_bound_to_the_original_identity() {
    struct QueryFailureBilling;

    #[async_trait]
    impl StoreBilling for QueryFailureBilling {
        async fn query_products(&self, _: &[String]) -> Result<Vec<StoreProduct>> {
            Ok(Vec::new())
        }

        async fn purchase(&self, _: &StoreProduct) -> Result<StoreTransaction> {
            panic!("purchase must not be reached")
        }

        async fn query_purchases(&self) -> Result<Vec<StoreTransaction>> {
            Err(Error::new(
                ErrorCode::StoreProblemError,
                "temporary native purchase query failure",
            ))
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
        Configuration::builder("appl_fake_key")
            .app_user_id("native-user")
            .proxy_url(&mock.url)
            .store_billing(Arc::new(QueryFailureBilling))
            .build()
            .unwrap(),
    )
    .unwrap();

    let sync_error = purchases.sync_purchases().await.unwrap_err();
    assert_eq!(sync_error.code, ErrorCode::StoreProblemError);

    let login_error = purchases.log_in("other-user").await.unwrap_err();
    assert_eq!(login_error.code, ErrorCode::OperationAlreadyInProgressError);
    let logout_error = purchases.log_out().await.unwrap_err();
    assert_eq!(
        logout_error.code,
        ErrorCode::OperationAlreadyInProgressError
    );
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
    let notifications = Arc::new(Mutex::new(Vec::new()));
    let listener_notifications = Arc::clone(&notifications);
    purchases.set_customer_info_listener(move |info| {
        listener_notifications
            .lock()
            .unwrap()
            .push(info.active_subscriptions().into_iter().collect::<Vec<_>>());
    });

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
    assert_eq!(
        *notifications.lock().unwrap(),
        vec![
            vec!["monthly".to_owned()],
            vec!["annual".to_owned(), "monthly".to_owned()]
        ]
    );
}

#[tokio::test]
async fn pending_sync_for_identified_user_is_not_a_restore() {
    // Arrange: automatic queue processing must not opt an identified user
    // into receipt sharing/transfer semantics.
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let owned = StoreTransaction {
        purchase_token: format!("{JWS_TOKEN_PREFIX}monthly-queue"),
        product_ids: vec!["monthly".into()],
        purchase_date: Utc::now(),
        transaction_id: Some("20000012".into()),
        store: Store::AppStore,
        price: None,
    };
    let (purchases, finish_calls) = purchases_against(
        &mock.url,
        FakeNativeBilling::with_catalog(native_catalog()).owning(vec![owned]),
    )
    .await;

    // Act
    let info = purchases.sync_pending_purchases().await.unwrap();

    // Assert
    assert!(info.entitlements.is_active("pro"));
    let receipts = mock.received_on("/v1/receipts");
    assert_eq!(receipts.len(), 1);
    let body = receipts[0].body.as_ref().unwrap();
    assert_eq!(body["is_restore"], false);
    assert_eq!(body["initiation_source"], "queue");
    assert_eq!(finish_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn pending_sync_reuses_native_persisted_receipt_attribution() {
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let transaction = StoreTransaction {
        purchase_token: format!("{JWS_TOKEN_PREFIX}monthly-recovered"),
        product_ids: vec!["monthly".into()],
        purchase_date: Utc::now(),
        transaction_id: Some("20000015".into()),
        store: Store::AppStore,
        price: None,
    };
    let receipt_context = StoreReceiptContext::new(
        Some("default".into()),
        Some("onboarding".into()),
        Some(TargetingResponse {
            rule_id: "rule".into(),
            revision: 7,
        }),
        Some(Price {
            amount_micros: 2_990_000,
            currency: "USD".into(),
        }),
        Some("P1M".into()),
        true,
    );
    let recovered =
        StorePurchase::from_transaction(transaction).with_receipt_context(receipt_context);
    let (purchases, finish_calls) = purchases_against(
        &mock.url,
        FakeNativeBilling::with_catalog(native_catalog()).owning_with_context(vec![recovered]),
    )
    .await;

    purchases.sync_pending_purchases().await.unwrap();

    let receipts = mock.received_on("/v1/receipts");
    assert_eq!(receipts.len(), 1);
    let body = receipts[0].body.as_ref().unwrap();
    assert_eq!(body["is_restore"], false);
    assert_eq!(body["initiation_source"], "purchase");
    assert_eq!(body["sdk_originated"], true);
    assert_eq!(body["presented_offering_identifier"], "default");
    assert_eq!(body["presented_placement_identifier"], "onboarding");
    assert_eq!(body["applied_targeting_rule"]["rule_id"], "rule");
    assert_eq!(body["applied_targeting_rule"]["revision"], 7);
    assert_eq!(body["price"], 2.99);
    assert_eq!(body["currency"], "USD");
    assert_eq!(body["normal_duration"], "P1M");
    assert_eq!(finish_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn pending_play_sync_uses_unsynced_active_purchases_source() {
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let owned = StoreTransaction {
        purchase_token: format!("{JWS_TOKEN_PREFIX}monthly-play-queue"),
        product_ids: vec!["monthly".into()],
        purchase_date: Utc::now(),
        transaction_id: Some("20000013".into()),
        store: Store::PlayStore,
        price: None,
    };
    let (purchases, _) = purchases_against(
        &mock.url,
        FakeNativeBilling::with_catalog(native_catalog()).owning(vec![owned]),
    )
    .await;

    purchases.sync_pending_purchases().await.unwrap();

    let receipts = mock.received_on("/v1/receipts");
    assert_eq!(receipts.len(), 1);
    let body = receipts[0].body.as_ref().unwrap();
    assert_eq!(body["is_restore"], false);
    assert_eq!(body["initiation_source"], "unsynced_active_purchases");
}

#[tokio::test]
async fn full_queue_sync_for_identified_user_does_not_use_restore_semantics() {
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let owned = StoreTransaction {
        purchase_token: format!("{JWS_TOKEN_PREFIX}monthly-full"),
        product_ids: vec!["monthly".into()],
        purchase_date: Utc::now(),
        transaction_id: Some("20000014".into()),
        store: Store::AppStore,
        price: None,
    };
    let (purchases, finish_calls) = purchases_against(
        &mock.url,
        FakeNativeBilling::with_catalog(native_catalog()).owning(vec![owned]),
    )
    .await;

    purchases.sync_purchases().await.unwrap();

    let receipts = mock.received_on("/v1/receipts");
    assert_eq!(receipts.len(), 1);
    let body = receipts[0].body.as_ref().unwrap();
    assert_eq!(body["is_restore"], false);
    assert_eq!(body["initiation_source"], "queue");
    assert_eq!(finish_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn full_queue_sync_for_anonymous_user_allows_store_account_sharing() {
    let mock = MockRevenueCat::with_default_catalog()
        .accept_store_tokens()
        .spawn()
        .await
        .unwrap();
    let owned = StoreTransaction {
        purchase_token: format!("{JWS_TOKEN_PREFIX}monthly-anonymous-full"),
        product_ids: vec!["monthly".into()],
        purchase_date: Utc::now(),
        transaction_id: Some("20000015".into()),
        store: Store::AppStore,
        price: None,
    };
    let billing = FakeNativeBilling::with_catalog(native_catalog()).owning(vec![owned]);
    let purchases = Purchases::configure(
        Configuration::builder("appl_fake_key")
            .proxy_url(&mock.url)
            .store_billing(Arc::new(billing))
            .build()
            .unwrap(),
    )
    .unwrap();

    purchases.sync_purchases().await.unwrap();

    assert!(purchases.is_anonymous());
    let receipts = mock.received_on("/v1/receipts");
    assert_eq!(receipts.len(), 1);
    let body = receipts[0].body.as_ref().unwrap();
    assert_eq!(body["is_restore"], true);
    assert_eq!(body["initiation_source"], "queue");
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
