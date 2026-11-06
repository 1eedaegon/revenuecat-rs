use super::*;

const SUBSCRIBER_FIXTURE: &str = include_str!("../../../tests/fixtures/subscriber.json");

fn fixture() -> CustomerInfo {
    let raw: serde_json::Value = serde_json::from_str(SUBSCRIBER_FIXTURE).unwrap();
    CustomerInfo::from_response(raw).unwrap()
}

#[test]
fn parses_purchases_js_fixture() {
    // Arrange / Act
    let info = fixture();

    // Assert
    assert_eq!(info.original_app_user_id, "someAppUserId");
    assert_eq!(info.subscriptions.len(), 2);
    assert_eq!(info.entitlements.all.len(), 2);
    assert_eq!(info.non_subscription_transactions.len(), 1);
    assert_eq!(
        info.management_url.as_deref(),
        Some("https://test-management-url.revenuecat.com")
    );
}

#[test]
fn entitlement_activeness_uses_request_date() {
    let info = fixture();
    assert!(info.entitlements.is_active("activeCatServices"));
    assert!(!info.entitlements.is_active("expiredCatServices"));
    assert!(!info.entitlements.is_active("nonexistent"));
}

#[test]
fn active_subscriptions_excludes_expired() {
    let info = fixture();
    let active = info.active_subscriptions();
    assert!(active.contains("black_f_friday_worten"));
    assert!(!active.contains("black_f_friday_worten_2"));
}

#[test]
fn all_purchased_product_ids_includes_non_subscriptions() {
    let info = fixture();
    assert!(info.all_purchased_product_ids().contains("consumable"));
}

#[test]
fn unknown_store_and_ownership_fall_back_gracefully() {
    // Arrange: minimal subscriber with unknown enum values.
    let raw = serde_json::json!({
        "request_date": "2024-01-22T13:23:07Z",
        "subscriber": {
            "original_app_user_id": "u",
            "first_seen": "2024-01-01T00:00:00Z",
            "subscriptions": {
                "p1": {"store": "some_future_store", "period_type": "weird", "expires_date": null}
            },
            "entitlements": {
                "pro": {"product_identifier": "p1"}
            }
        }
    });

    // Act
    let info = CustomerInfo::from_response(raw).unwrap();

    // Assert: lenient parsing like the official SDKs.
    assert_eq!(info.subscriptions["p1"].store, Store::Unknown);
    assert_eq!(info.subscriptions["p1"].period_type, PeriodType::Unknown);
    assert!(
        info.entitlements.is_active("pro"),
        "no expires_date means lifetime-active"
    );
}

#[test]
fn family_shared_ownership_parses() {
    let info = fixture();
    assert_eq!(
        info.subscriptions["black_f_friday_worten_2"].ownership_type,
        Some(OwnershipType::FamilyShared)
    );
}

#[test]
fn prepaid_subscriptions_and_entitlements_do_not_renew() {
    // Arrange: RevenueCat models prepaid periods as active until their
    // expiration, but they never auto-renew.
    let raw = serde_json::json!({
        "request_date": "2026-11-05T00:00:00Z",
        "subscriber": {
            "original_app_user_id": "prepaid-user",
            "first_seen": "2026-11-01T00:00:00Z",
            "subscriptions": {
                "prepaid-month": {
                    "store": "play_store",
                    "period_type": "prepaid",
                    "purchase_date": "2026-11-01T00:00:00Z",
                    "expires_date": "2026-12-01T00:00:00Z"
                }
            },
            "entitlements": {
                "pro": {
                    "product_identifier": "prepaid-month",
                    "purchase_date": "2026-11-01T00:00:00Z",
                    "expires_date": "2026-12-01T00:00:00Z"
                }
            }
        }
    });

    // Act
    let info = CustomerInfo::from_response(raw).unwrap();

    // Assert
    assert!(!info.subscriptions["prepaid-month"].will_renew);
    assert!(!info.entitlements.all["pro"].will_renew);
}

#[test]
fn assigned_ownership_remains_available_through_raw_accessors() {
    // Arrange: keep the existing exhaustive OwnershipType enum source
    // compatible while retaining the newer ASSIGNED wire value.
    let raw = serde_json::json!({
        "request_date": "2026-11-05T00:00:00Z",
        "subscriber": {
            "original_app_user_id": "organization-user",
            "first_seen": "2026-11-01T00:00:00Z",
            "subscriptions": {
                "organization-plan": {
                    "store": "external",
                    "period_type": "normal",
                    "ownership_type": "ASSIGNED",
                    "display_name": "Organization Pro",
                    "product_plan_identifier": "organization-annual",
                    "management_url": "https://example.com/manage"
                }
            },
            "entitlements": {
                "pro": {
                    "product_identifier": "organization-plan",
                    "future_server_field": "retained"
                }
            }
        }
    });

    // Act
    let info = CustomerInfo::from_response(raw).unwrap();

    // Assert: legacy callers still receive Unknown, while callers that
    // understand the current server value can distinguish ASSIGNED.
    assert_eq!(
        info.subscriptions["organization-plan"].ownership_type,
        Some(OwnershipType::Unknown)
    );
    assert_eq!(
        info.subscription_ownership_type_raw("organization-plan"),
        Some("ASSIGNED")
    );
    assert_eq!(info.entitlement_ownership_type_raw("pro"), Some("ASSIGNED"));
    assert_eq!(
        info.raw_subscription("organization-plan").unwrap()["display_name"],
        "Organization Pro"
    );
    assert_eq!(
        info.raw_subscription("organization-plan").unwrap()["product_plan_identifier"],
        "organization-annual"
    );
    assert_eq!(
        info.raw_entitlement("pro").unwrap()["future_server_field"],
        "retained"
    );
}

#[test]
fn customer_info_serialization_preserves_the_legacy_js_contract() {
    // Arrange: Galaxy and ASSIGNED are valid current backend values, but
    // the public 1.x JavaScript unions cannot represent either one.
    let raw = serde_json::json!({
        "request_date": "2026-11-05T00:00:00Z",
        "subscriber": {
            "original_app_user_id": "legacy-js-user",
            "first_seen": "2026-11-01T00:00:00Z",
            "subscriptions": {
                "galaxy-plan": {
                    "store": "galaxy",
                    "period_type": "normal",
                    "ownership_type": "ASSIGNED"
                }
            },
            "entitlements": {
                "pro": {
                    "product_identifier": "galaxy-plan"
                }
            },
            "non_subscriptions": {
                "galaxy-coins": [{
                    "id": "transaction-1",
                    "store": "galaxy",
                    "purchase_date": "2026-11-04T00:00:00Z"
                }]
            }
        }
    });
    let info = CustomerInfo::from_response(raw).unwrap();

    assert_eq!(info.subscriptions["galaxy-plan"].store, Store::Galaxy);
    assert_eq!(
        info.subscriptions["galaxy-plan"].ownership_type,
        Some(OwnershipType::Unknown)
    );
    assert_eq!(info.entitlements.all["pro"].store, Store::Galaxy);
    assert_eq!(info.non_subscription_transactions[0].store, Store::Galaxy);

    // Act
    let serialized = serde_json::to_value(&info).unwrap();

    // Assert: typed fields stay inside the legacy unions.
    assert_eq!(
        serialized["subscriptions"]["galaxy-plan"]["store"],
        "unknown"
    );
    assert_eq!(
        serialized["subscriptions"]["galaxy-plan"]["ownership_type"],
        serde_json::Value::Null
    );
    assert_eq!(serialized["entitlements"]["all"]["pro"]["store"], "unknown");
    assert_eq!(
        serialized["entitlements"]["all"]["pro"]["ownership_type"],
        serde_json::Value::Null
    );
    assert_eq!(
        serialized["non_subscription_transactions"][0]["store"],
        "unknown"
    );

    // The raw response remains the lossless escape hatch for new values.
    assert_eq!(
        serialized["raw"]["subscriber"]["subscriptions"]["galaxy-plan"]["store"],
        "galaxy"
    );
    assert_eq!(
        serialized["raw"]["subscriber"]["subscriptions"]["galaxy-plan"]["ownership_type"],
        "ASSIGNED"
    );
}
