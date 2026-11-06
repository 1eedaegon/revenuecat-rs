use std::sync::atomic::{AtomicUsize, Ordering};

use super::store_options::{
    billed_base_plan_of_earlier_launch, remember_store_purchase_options,
    store_purchase_option_for_package, store_purchase_options, KnownStorePackages,
    StorePurchaseOptionRegistry, StoredPackagePurchaseOption, MAX_KNOWN_PACKAGE_FINGERPRINTS,
    MAX_PACKAGE_FINGERPRINTS_PER_USER,
};
use super::*;
use crate::models::{OfferingsResponse, Price, ProductType};
use revenuecat_mock::MockRevenueCat;

#[test]
fn purchase_preflight_rejects_blank_attribution_and_plan_identifiers() {
    let product = StoreProduct {
        identifier: "premium".into(),
        product_type: ProductType::Subscription,
        title: "Premium".into(),
        description: None,
        price: Price {
            amount_micros: 4_990_000,
            currency: "USD".into(),
        },
        subscription_period: Some("P1M".into()),
        trial: None,
        intro_price: None,
    };
    let targeting = TargetingResponse {
        rule_id: " ".into(),
        revision: 1,
    };
    let plan = StorePurchaseOption::for_plan("\t");
    let invalid_contexts = [
        validate_purchase_input(&product, Some(""), None, None, None),
        validate_purchase_input(&product, None, Some(" "), None, None),
        validate_purchase_input(&product, None, None, Some(&targeting), None),
        validate_purchase_input(&product, None, None, None, Some(&plan)),
    ];

    for result in invalid_contexts {
        assert_eq!(result.unwrap_err().code, ErrorCode::ConfigurationError);
    }
}

#[test]
fn purchases_debug_redacts_the_app_user_id() {
    let purchases = Purchases::configure(
        Configuration::builder("test_debug_redaction")
            .app_user_id("private-user@example.com")
            .build()
            .unwrap(),
    )
    .unwrap();

    let debug = format!("{purchases:?}");

    assert!(!debug.contains("private-user@example.com"));
    assert!(debug.contains("[REDACTED]"));
}

#[test]
fn backend_confirmed_earlier_play_base_plan_is_detected_for_the_exact_order() {
    let customer_info = CustomerInfo::from_response(serde_json::json!({
        "request_date": "2026-11-05T00:00:00Z",
        "subscriber": {
            "original_app_user_id": "user-a",
            "first_seen": "2026-01-01T00:00:00Z",
            "subscriptions": {
                "premium": {
                    "purchase_date": "2026-11-05T00:00:00Z",
                    "store": "play_store",
                    "store_transaction_id": "GPA.exact-order",
                    "product_plan_identifier": "annual-base"
                }
            }
        }
    }))
    .unwrap();
    let transaction = StoreTransaction {
        purchase_token: "play-token".into(),
        product_ids: vec!["premium".into()],
        purchase_date: chrono::Utc::now(),
        transaction_id: Some("GPA.exact-order".into()),
        store: Store::PlayStore,
        price: None,
    };
    let launched_plans =
        std::collections::BTreeSet::from(["monthly-base".to_owned(), "annual-base".to_owned()]);

    assert_eq!(
        billed_base_plan_of_earlier_launch(
            &transaction,
            &customer_info,
            Some("monthly-base"),
            Some(&launched_plans),
        ),
        Some("annual-base".to_owned())
    );

    let different_order = StoreTransaction {
        transaction_id: Some("GPA.different-order".into()),
        ..transaction.clone()
    };
    assert_eq!(
        billed_base_plan_of_earlier_launch(
            &different_order,
            &customer_info,
            Some("monthly-base"),
            Some(&launched_plans),
        ),
        None
    );
    assert_eq!(
        billed_base_plan_of_earlier_launch(
            &transaction,
            &customer_info,
            Some("annual-base"),
            Some(&launched_plans),
        ),
        None
    );
}

#[test]
fn current_offering_plan_identifier_becomes_a_store_purchase_option() {
    let response: OfferingsResponse = serde_json::from_value(serde_json::json!({
        "current_offering_id": "default",
        "offerings": [{
            "identifier": "default",
            "packages": [{
                "identifier": "$rc_monthly",
            "platform_product_identifier": "monthly",
            "platform_product_plan_identifier": "monthly-base"
            }]
        }]
    }))
    .unwrap();

    let product = StoreProduct {
        identifier: "monthly".into(),
        product_type: ProductType::Subscription,
        title: "Monthly".into(),
        description: None,
        price: Price {
            amount_micros: 4_990_000,
            currency: "USD".into(),
        },
        subscription_period: Some("P1M".into()),
        trial: None,
        intro_price: None,
    };
    let offerings = Offerings::resolve(
        &response,
        &BTreeMap::from([("monthly".to_owned(), product)]),
    );
    let options = store_purchase_options(&response, &offerings);

    assert_eq!(
        options[0]
            .option
            .as_ref()
            .unwrap()
            .platform_product_plan_identifier(),
        Some("monthly-base")
    );
}

#[test]
fn identical_package_snapshot_with_different_plans_is_rejected() {
    let package = Package {
        identifier: "$rc_monthly".into(),
        package_type: crate::models::PackageType::Monthly,
        store_product: StoreProduct {
            identifier: "premium".into(),
            product_type: ProductType::Subscription,
            title: "Premium".into(),
            description: None,
            price: Price {
                amount_micros: 4_990_000,
                currency: "USD".into(),
            },
            subscription_period: Some("P1M".into()),
            trial: None,
            intro_price: None,
        },
        presented_offering_context: crate::models::PresentedOfferingContext {
            offering_identifier: Some("default".into()),
            placement_identifier: None,
            targeting: None,
        },
    };
    let options = vec![
        StoredPackagePurchaseOption::new(
            &package,
            Some(StorePurchaseOption::for_plan("monthly-v1")),
        ),
        StoredPackagePurchaseOption::new(
            &package,
            Some(StorePurchaseOption::for_plan("monthly-v2")),
        ),
    ];

    let error = store_purchase_option_for_package(&options, &package, true).unwrap_err();

    assert_eq!(error.code, ErrorCode::ProductNotAvailableForPurchaseError);
}

#[test]
fn current_planless_package_snapshot_is_a_valid_purchase() {
    let package = Package {
        identifier: "lifetime".into(),
        package_type: crate::models::PackageType::Lifetime,
        store_product: StoreProduct {
            identifier: "lifetime".into(),
            product_type: ProductType::NonConsumable,
            title: "Lifetime".into(),
            description: None,
            price: Price {
                amount_micros: 49_990_000,
                currency: "USD".into(),
            },
            subscription_period: None,
            trial: None,
            intro_price: None,
        },
        presented_offering_context: crate::models::PresentedOfferingContext {
            offering_identifier: Some("default".into()),
            placement_identifier: None,
            targeting: None,
        },
    };
    let options = vec![StoredPackagePurchaseOption::new(&package, None)];

    assert_eq!(
        store_purchase_option_for_package(&options, &package, true).unwrap(),
        None
    );
}

#[test]
fn manually_constructed_packages_remain_valid_across_offering_snapshots() {
    let package = Package {
        identifier: "$rc_monthly".into(),
        package_type: crate::models::PackageType::Monthly,
        store_product: StoreProduct {
            identifier: "premium".into(),
            product_type: ProductType::Subscription,
            title: "Premium".into(),
            description: None,
            price: Price {
                amount_micros: 4_990_000,
                currency: "USD".into(),
            },
            subscription_period: Some("P1M".into()),
            trial: None,
            intro_price: None,
        },
        presented_offering_context: crate::models::PresentedOfferingContext {
            offering_identifier: Some("default".into()),
            placement_identifier: None,
            targeting: None,
        },
    };

    assert_eq!(
        store_purchase_option_for_package(&[], &package, false).unwrap(),
        None,
        "1.x callers can still purchase directly constructed packages"
    );

    let options = vec![StoredPackagePurchaseOption::new(&package, None)];
    let mut unknown_package = package.clone();
    unknown_package.identifier = "different-package".into();
    assert_eq!(
        store_purchase_option_for_package(&options, &unknown_package, false).unwrap(),
        None,
        "a never-issued manual package must not depend on prior get_offerings calls"
    );
}

#[test]
fn sdk_package_from_a_previous_identity_is_not_treated_as_manual() {
    let sdk_package = Package {
        identifier: "$rc_monthly".into(),
        package_type: crate::models::PackageType::Monthly,
        store_product: StoreProduct {
            identifier: "premium".into(),
            product_type: ProductType::Subscription,
            title: "Premium".into(),
            description: None,
            price: Price {
                amount_micros: 4_990_000,
                currency: "USD".into(),
            },
            subscription_period: Some("P1M".into()),
            trial: None,
            intro_price: None,
        },
        presented_offering_context: crate::models::PresentedOfferingContext {
            offering_identifier: Some("default".into()),
            placement_identifier: None,
            targeting: None,
        },
    };
    let mut registry = StorePurchaseOptionRegistry::default();
    registry.remember_for_user(
        "user-a",
        vec![StoredPackagePurchaseOption::new(
            &sdk_package,
            Some(StorePurchaseOption::for_plan("monthly-base")),
        )],
    );
    assert_eq!(
        registry
            .purchase_option_for_package("user-a", &sdk_package)
            .unwrap()
            .unwrap()
            .platform_product_plan_identifier(),
        Some("monthly-base")
    );

    registry.clear_identity_scoped_options();
    for user in ["user-b", "user-a"] {
        let error = registry
            .purchase_option_for_package(user, &sdk_package)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ProductNotAvailableForPurchaseError);
    }

    let mut user_b_package = sdk_package.clone();
    user_b_package.identifier = "$rc_annual".into();
    registry.remember_for_user(
        "user-b",
        vec![StoredPackagePurchaseOption::new(
            &user_b_package,
            Some(StorePurchaseOption::for_plan("annual-base")),
        )],
    );
    assert!(registry
        .purchase_option_for_package("user-b", &user_b_package)
        .is_ok());

    registry.clear_identity_scoped_options();
    let mut manual_package = sdk_package;
    manual_package.identifier = "manually-created".into();
    assert_eq!(
        registry
            .purchase_option_for_package("user-b", &manual_package)
            .unwrap(),
        None
    );
}

#[test]
fn saturated_package_provenance_fails_closed_for_unknown_fingerprints() {
    let base_package = Package {
        identifier: "package-0".into(),
        package_type: crate::models::PackageType::Custom("package-0".into()),
        store_product: StoreProduct {
            identifier: "product-0".into(),
            product_type: ProductType::Subscription,
            title: "Product 0".into(),
            description: None,
            price: Price {
                amount_micros: 1_000_000,
                currency: "USD".into(),
            },
            subscription_period: Some("P1M".into()),
            trial: None,
            intro_price: None,
        },
        presented_offering_context: crate::models::PresentedOfferingContext::default(),
    };
    let incoming = (0..=MAX_KNOWN_PACKAGE_FINGERPRINTS)
        .map(|index| {
            let mut package = base_package.clone();
            package.identifier = format!("package-{index}");
            StoredPackagePurchaseOption::new(&package, None)
        })
        .collect::<Vec<_>>();
    let mut known_packages = KnownStorePackages::default();
    known_packages.remember(&incoming);
    let mut unknown_package = base_package;
    unknown_package.identifier = "never-seen".into();

    assert!(known_packages.contains(&unknown_package));
}

#[test]
fn remembered_package_snapshots_are_bounded_and_plan_conflicts_stay_fail_closed() {
    let base_package = Package {
        identifier: "package-0".into(),
        package_type: crate::models::PackageType::Custom("package-0".into()),
        store_product: StoreProduct {
            identifier: "product-0".into(),
            product_type: ProductType::Subscription,
            title: "Product 0".into(),
            description: None,
            price: Price {
                amount_micros: 1_000_000,
                currency: "USD".into(),
            },
            subscription_period: Some("P1M".into()),
            trial: None,
            intro_price: None,
        },
        presented_offering_context: crate::models::PresentedOfferingContext {
            offering_identifier: Some("default".into()),
            placement_identifier: None,
            targeting: None,
        },
    };
    let snapshots = (0..=MAX_PACKAGE_FINGERPRINTS_PER_USER)
        .map(|index| {
            let mut package = base_package.clone();
            package.identifier = format!("package-{index}");
            package.store_product.identifier = format!("product-{index}");
            StoredPackagePurchaseOption::new(
                &package,
                Some(StorePurchaseOption::for_plan(format!("plan-{index}"))),
            )
        })
        .collect();
    let mut remembered = Vec::new();

    remember_store_purchase_options(&mut remembered, snapshots);

    assert_eq!(remembered.len(), MAX_PACKAGE_FINGERPRINTS_PER_USER);

    remember_store_purchase_options(
        &mut remembered,
        vec![StoredPackagePurchaseOption::new(
            &base_package,
            Some(StorePurchaseOption::for_plan("changed-plan")),
        )],
    );

    assert_eq!(remembered.len(), MAX_PACKAGE_FINGERPRINTS_PER_USER + 1);
    let error = store_purchase_option_for_package(&remembered, &base_package, true).unwrap_err();
    assert_eq!(error.code, ErrorCode::ProductNotAvailableForPurchaseError);
}

#[test]
fn customer_info_response_never_notifies_or_populates_the_current_other_user() {
    let purchases = Purchases::configure(
        Configuration::builder("test_identity_race")
            .app_user_id("user-a")
            .build()
            .unwrap(),
    )
    .unwrap();
    let notifications = Arc::new(AtomicUsize::new(0));
    let listener_notifications = Arc::clone(&notifications);
    purchases.set_customer_info_listener(move |_| {
        listener_notifications.fetch_add(1, Ordering::SeqCst);
    });
    purchases.inner.identity.switch_user("user-b");
    let info = CustomerInfo::from_response(serde_json::json!({
        "request_date": "2026-01-01T00:00:00Z",
        "subscriber": {
            "original_app_user_id": "user-a",
            "first_seen": "2026-01-01T00:00:00Z"
        }
    }))
    .unwrap();

    purchases.store_and_notify_for_user("user-a", &info);

    assert!(purchases
        .inner
        .cache
        .cached_customer_info("user-a")
        .is_some());
    assert!(purchases
        .inner
        .cache
        .cached_customer_info("user-b")
        .is_none());
    assert_eq!(notifications.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn identity_changes_wait_for_the_active_identity_operation() {
    let purchases = Purchases::configure(
        Configuration::builder("test_identity_serialization")
            .app_user_id("user-a")
            .build()
            .unwrap(),
    )
    .unwrap();
    let active_operation = purchases.inner.identity_operation.lock().await;

    let result =
        tokio::time::timeout(std::time::Duration::from_millis(20), purchases.log_out()).await;

    assert!(
        result.is_err(),
        "logOut must wait for the prior identity operation"
    );
    drop(active_operation);
}

#[tokio::test]
async fn same_user_login_returns_the_captured_user_when_logout_is_queued() {
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("test_identity_same_user")
            .app_user_id("user-a")
            .proxy_url(&server.url)
            .build()
            .unwrap(),
    )
    .unwrap();
    let active_operation = purchases.inner.identity_operation.lock().await;

    let login_purchases = purchases.clone();
    let login = tokio::spawn(async move { login_purchases.log_in("user-a").await });
    tokio::task::yield_now().await;
    let logout_purchases = purchases.clone();
    let logout = tokio::spawn(async move { logout_purchases.log_out().await });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    drop(active_operation);

    let (login_info, created) = login.await.unwrap().unwrap();
    logout.await.unwrap().unwrap();

    assert!(!created);
    assert_eq!(login_info.original_app_user_id, "user-a");
}

#[tokio::test]
async fn failed_identity_commit_keeps_login_on_the_original_user() {
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("test_identity_commit_login")
            .app_user_id("user-a")
            .proxy_url(&server.url)
            .build()
            .unwrap(),
    )
    .unwrap();

    let error = purchases
        .log_in_with_identity_transaction(
            "user-b",
            |_, _| Ok(()),
            |_| {
                Err(Error::new(
                    ErrorCode::ConfigurationError,
                    "simulated durable identity failure",
                ))
            },
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::ConfigurationError);
    assert_eq!(purchases.app_user_id(), "user-a");
    assert_eq!(
        purchases
            .ensure_receipt_operations_ready()
            .unwrap_err()
            .code,
        ErrorCode::OperationAlreadyInProgressError
    );
}

#[tokio::test]
async fn failed_login_commit_only_allows_the_exact_transition_to_resume() {
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("test_identity_commit_login_resume")
            .app_user_id("user-a")
            .proxy_url(&server.url)
            .build()
            .unwrap(),
    )
    .unwrap();

    purchases
        .log_in_with_identity_transaction(
            "user-b",
            |_, _| Ok(()),
            |_| {
                Err(Error::new(
                    ErrorCode::ConfigurationError,
                    "simulated durable identity failure",
                ))
            },
        )
        .await
        .unwrap_err();

    let conflicting_prepare_calls = Arc::new(AtomicUsize::new(0));
    let captured_calls = Arc::clone(&conflicting_prepare_calls);
    let error = purchases
        .log_in_with_identity_transaction(
            "user-c",
            move |_, _| {
                captured_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |_| Ok(()),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::OperationAlreadyInProgressError);
    assert_eq!(conflicting_prepare_calls.load(Ordering::SeqCst), 0);

    let (_, created) = purchases
        .log_in_with_identity_transaction("user-b", |_, _| Ok(()), |_| Ok(()))
        .await
        .unwrap();

    assert!(!created);
    assert_eq!(purchases.app_user_id(), "user-b");
    purchases.ensure_receipt_operations_ready().unwrap();
}

#[tokio::test]
async fn failed_login_prepare_also_requires_the_exact_transition_to_resume() {
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("test_identity_prepare_login_resume")
            .app_user_id("user-a")
            .proxy_url(&server.url)
            .build()
            .unwrap(),
    )
    .unwrap();

    let error = purchases
        .log_in_with_identity_transaction(
            "user-b",
            |_, _| {
                Err(Error::new(
                    ErrorCode::ConfigurationError,
                    "simulated durable identity prepare failure",
                ))
            },
            |_| Ok(()),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::ConfigurationError);
    assert_eq!(purchases.app_user_id(), "user-a");
    assert_eq!(
        purchases
            .ensure_receipt_operations_ready()
            .unwrap_err()
            .code,
        ErrorCode::OperationAlreadyInProgressError
    );

    let conflicting_error = purchases
        .log_in_with_identity_transaction("user-c", |_, _| Ok(()), |_| Ok(()))
        .await
        .unwrap_err();
    assert_eq!(
        conflicting_error.code,
        ErrorCode::OperationAlreadyInProgressError
    );

    purchases
        .log_in_with_identity_transaction("user-b", |_, _| Ok(()), |_| Ok(()))
        .await
        .unwrap();
    assert_eq!(purchases.app_user_id(), "user-b");
    purchases.ensure_receipt_operations_ready().unwrap();
}

#[tokio::test]
async fn invalid_tracked_login_is_rejected_without_wedging_identity_operations() {
    let purchases = Purchases::configure(
        Configuration::builder("test_invalid_tracked_login")
            .app_user_id("user-a")
            .build()
            .unwrap(),
    )
    .unwrap();
    let prepare_calls = Arc::new(AtomicUsize::new(0));

    for invalid_app_user_id in [" ".to_owned(), "x".repeat(1_025)] {
        let captured_calls = Arc::clone(&prepare_calls);
        let error = purchases
            .log_in_with_identity_transaction(
                &invalid_app_user_id,
                move |_, _| {
                    captured_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
                |_| Ok(()),
            )
            .await
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::InvalidAppUserIdError);
        purchases.ensure_receipt_operations_ready().unwrap();
    }
    assert_eq!(prepare_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn failed_identity_commit_keeps_logout_on_the_original_user() {
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("test_identity_commit_logout")
            .app_user_id("user-a")
            .proxy_url(&server.url)
            .build()
            .unwrap(),
    )
    .unwrap();
    let attempted_identity = Arc::new(Mutex::new(None));
    let captured_identity = Arc::clone(&attempted_identity);

    let error = purchases
        .log_out_with_identity_transaction(
            |_, _| Ok(()),
            move |new_app_user_id| {
                *captured_identity.lock().unwrap() = Some(new_app_user_id.to_owned());
                Err(Error::new(
                    ErrorCode::ConfigurationError,
                    "simulated durable identity failure",
                ))
            },
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::ConfigurationError);
    assert_eq!(purchases.app_user_id(), "user-a");
    assert!(attempted_identity
        .lock()
        .unwrap()
        .as_deref()
        .is_some_and(|identity| identity.starts_with("$RCAnonymousID:")));
    assert_eq!(
        purchases
            .ensure_receipt_operations_ready()
            .unwrap_err()
            .code,
        ErrorCode::OperationAlreadyInProgressError
    );
}

#[tokio::test]
async fn failed_logout_commit_only_allows_the_exact_anonymous_id_to_resume() {
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("test_identity_commit_logout_resume")
            .app_user_id("user-a")
            .proxy_url(&server.url)
            .build()
            .unwrap(),
    )
    .unwrap();
    let attempted_identity = Arc::new(Mutex::new(None));
    let captured_identity = Arc::clone(&attempted_identity);

    purchases
        .log_out_with_identity_transaction(
            |_, _| Ok(()),
            move |new_app_user_id| {
                *captured_identity.lock().unwrap() = Some(new_app_user_id.to_owned());
                Err(Error::new(
                    ErrorCode::ConfigurationError,
                    "simulated durable identity failure",
                ))
            },
        )
        .await
        .unwrap_err();

    let error = purchases.log_out().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::OperationAlreadyInProgressError);

    let pending_anonymous_id = attempted_identity.lock().unwrap().clone().unwrap();
    purchases
        .resume_log_out_with_identity_transaction(&pending_anonymous_id, |_, _| Ok(()), |_| Ok(()))
        .await
        .unwrap();

    assert_eq!(purchases.app_user_id(), pending_anonymous_id);
    purchases.ensure_receipt_operations_ready().unwrap();
}

#[tokio::test]
async fn failed_logout_prepare_only_allows_the_exact_anonymous_id_to_resume() {
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("test_identity_prepare_logout_resume")
            .app_user_id("user-a")
            .proxy_url(&server.url)
            .build()
            .unwrap(),
    )
    .unwrap();
    let attempted_identity = Arc::new(Mutex::new(None));
    let captured_identity = Arc::clone(&attempted_identity);

    purchases
        .log_out_with_identity_transaction(
            move |_, new_app_user_id| {
                *captured_identity.lock().unwrap() = Some(new_app_user_id.to_owned());
                Err(Error::new(
                    ErrorCode::ConfigurationError,
                    "simulated durable identity prepare failure",
                ))
            },
            |_| Ok(()),
        )
        .await
        .unwrap_err();

    let error = purchases.log_out().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::OperationAlreadyInProgressError);

    let pending_anonymous_id = attempted_identity.lock().unwrap().clone().unwrap();
    purchases
        .resume_log_out_with_identity_transaction(&pending_anonymous_id, |_, _| Ok(()), |_| Ok(()))
        .await
        .unwrap();

    assert_eq!(purchases.app_user_id(), pending_anonymous_id);
    purchases.ensure_receipt_operations_ready().unwrap();
}

#[tokio::test]
async fn tracked_logout_retry_reuses_the_hidden_pending_anonymous_id() {
    let server = MockRevenueCat::with_default_catalog()
        .spawn()
        .await
        .unwrap();
    let purchases = Purchases::configure(
        Configuration::builder("test_identity_prepare_logout_retry")
            .app_user_id("user-a")
            .proxy_url(&server.url)
            .build()
            .unwrap(),
    )
    .unwrap();
    let first_identity = Arc::new(Mutex::new(None));
    let captured_first_identity = Arc::clone(&first_identity);

    purchases
        .log_out_with_identity_transaction(
            move |_, pending_app_user_id| {
                *captured_first_identity.lock().unwrap() = Some(pending_app_user_id.to_owned());
                Err(Error::new(
                    ErrorCode::ConfigurationError,
                    "simulated durable identity prepare failure",
                ))
            },
            |_| Ok(()),
        )
        .await
        .unwrap_err();

    let retried_identity = Arc::new(Mutex::new(None));
    let captured_retried_identity = Arc::clone(&retried_identity);
    purchases
        .log_out_with_identity_transaction(
            move |_, pending_app_user_id| {
                *captured_retried_identity.lock().unwrap() = Some(pending_app_user_id.to_owned());
                Ok(())
            },
            |_| Ok(()),
        )
        .await
        .unwrap();

    assert_eq!(
        retried_identity.lock().unwrap().as_ref(),
        first_identity.lock().unwrap().as_ref()
    );
    assert_eq!(
        purchases.app_user_id(),
        first_identity.lock().unwrap().clone().unwrap()
    );
    purchases.ensure_receipt_operations_ready().unwrap();
}
