use std::collections::{BTreeMap, BTreeSet};

use crate::error::{Error, ErrorCode, Result};
use crate::models::{
    CustomerInfo, Offerings, OfferingsResponse, Package, Store, StoreProduct, StoreProductQuery,
    StorePurchaseOption, StoreTransaction,
};

pub(super) const MAX_PACKAGE_FINGERPRINTS_PER_USER: usize = 256;
pub(super) const MAX_KNOWN_PACKAGE_FINGERPRINTS: usize = 256;

/// Detects the Play Billing edge case handled by purchases-android 11.1:
/// Play does not return a base plan on a purchase, so a delayed transaction
/// can arrive stamped with the currently launched option. RevenueCat's backend
/// is authoritative for the billed plan and order ID.
pub(super) fn billed_base_plan_of_earlier_launch(
    transaction: &StoreTransaction,
    customer_info: &CustomerInfo,
    launched_base_plan_id: Option<&str>,
    plans_launched_for_product: Option<&BTreeSet<String>>,
) -> Option<String> {
    if transaction.store != Store::PlayStore {
        return None;
    }
    let launched_base_plan_id = launched_base_plan_id?;
    let product_id = transaction.product_ids.first()?;
    let order_id = transaction.transaction_id.as_deref()?;
    let subscription = customer_info.raw_subscription(product_id)?;
    if subscription
        .get("store_transaction_id")
        .and_then(serde_json::Value::as_str)
        != Some(order_id)
    {
        return None;
    }
    let billed_base_plan_id = subscription
        .get("product_plan_identifier")
        .and_then(serde_json::Value::as_str)?;
    if billed_base_plan_id == launched_base_plan_id
        || !plans_launched_for_product?.contains(billed_base_plan_id)
    {
        return None;
    }
    Some(billed_base_plan_id.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StoredPackagePurchaseOption {
    offering_identifier: Option<String>,
    package_identifier: String,
    store_product: StoreProduct,
    placement_identifier: Option<String>,
    targeting: Option<(String, i64)>,
    pub(super) option: Option<StorePurchaseOption>,
}

impl StoredPackagePurchaseOption {
    pub(super) fn new(package: &Package, option: Option<StorePurchaseOption>) -> Self {
        Self {
            offering_identifier: package
                .presented_offering_context
                .offering_identifier
                .clone(),
            package_identifier: package.identifier.clone(),
            store_product: package.store_product.clone(),
            placement_identifier: package
                .presented_offering_context
                .placement_identifier
                .clone(),
            targeting: package
                .presented_offering_context
                .targeting
                .as_ref()
                .map(|targeting| (targeting.rule_id.clone(), targeting.revision)),
            option,
        }
    }

    fn matches(&self, package: &Package) -> bool {
        self.offering_identifier
            == package
                .presented_offering_context
                .offering_identifier
                .clone()
            && self.package_identifier == package.identifier
            && self.store_product == package.store_product
            && self.placement_identifier
                == package
                    .presented_offering_context
                    .placement_identifier
                    .clone()
            && self.targeting
                == package
                    .presented_offering_context
                    .targeting
                    .as_ref()
                    .map(|targeting| (targeting.rule_id.clone(), targeting.revision))
    }

    fn has_same_package_fingerprint(&self, other: &Self) -> bool {
        self.offering_identifier == other.offering_identifier
            && self.package_identifier == other.package_identifier
            && self.store_product == other.store_product
            && self.placement_identifier == other.placement_identifier
            && self.targeting == other.targeting
    }
}

/// Bounded provenance retained across identity changes. Once saturated, the
/// registry treats every package as SDK-issued so an evicted stale package can
/// never fall back to the legacy planless purchase path.
#[derive(Debug, Default)]
pub(super) struct KnownStorePackages {
    packages: Vec<StoredPackagePurchaseOption>,
    saturated: bool,
}

impl KnownStorePackages {
    pub(super) fn remember(&mut self, incoming: &[StoredPackagePurchaseOption]) {
        for candidate in incoming {
            if self
                .packages
                .iter()
                .any(|known| known.has_same_package_fingerprint(candidate))
            {
                continue;
            }
            if self.packages.len() == MAX_KNOWN_PACKAGE_FINGERPRINTS {
                self.saturated = true;
                continue;
            }
            self.packages.push(candidate.clone());
        }
    }

    pub(super) fn contains(&self, package: &Package) -> bool {
        self.saturated || self.packages.iter().any(|known| known.matches(package))
    }
}

/// Atomically pairs the current identity's plan snapshots with the bounded
/// cross-identity provenance needed to reject stale SDK-issued packages.
#[derive(Debug, Default)]
pub(super) struct StorePurchaseOptionRegistry {
    options_by_user: BTreeMap<String, Vec<StoredPackagePurchaseOption>>,
    known_packages: KnownStorePackages,
}

impl StorePurchaseOptionRegistry {
    pub(super) fn remember_for_user(
        &mut self,
        app_user_id: &str,
        incoming: Vec<StoredPackagePurchaseOption>,
    ) {
        self.known_packages.remember(&incoming);
        remember_store_purchase_options(
            self.options_by_user
                .entry(app_user_id.to_owned())
                .or_default(),
            incoming,
        );
    }

    pub(super) fn purchase_option_for_package(
        &self,
        app_user_id: &str,
        package: &Package,
    ) -> Result<Option<StorePurchaseOption>> {
        store_purchase_option_for_package(
            self.options_by_user
                .get(app_user_id)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            package,
            self.known_packages.contains(package),
        )
    }

    pub(super) fn clear_identity_scoped_options(&mut self) {
        self.options_by_user.clear();
    }
}

/// Remembers a bounded number of package generations while retaining one
/// conflicting option for any known fingerprint. Two entries are sufficient
/// for `store_purchase_option_for_package` to fail closed after a plan change;
/// further generations add no safety signal and are discarded.
pub(super) fn remember_store_purchase_options(
    stored: &mut Vec<StoredPackagePurchaseOption>,
    incoming: Vec<StoredPackagePurchaseOption>,
) {
    let mut fingerprint_count = stored
        .iter()
        .enumerate()
        .filter(|(index, candidate)| {
            !stored[..*index]
                .iter()
                .any(|prior| prior.has_same_package_fingerprint(candidate))
        })
        .count();

    for candidate in incoming {
        let matching = stored
            .iter()
            .filter(|stored| stored.has_same_package_fingerprint(&candidate))
            .collect::<Vec<_>>();
        if matching
            .iter()
            .any(|stored| stored.option == candidate.option)
        {
            continue;
        }
        if !matching.is_empty() {
            if matching.len() == 1 {
                stored.push(candidate);
            }
            continue;
        }
        if fingerprint_count < MAX_PACKAGE_FINGERPRINTS_PER_USER {
            stored.push(candidate);
            fingerprint_count += 1;
        }
    }
}

pub(super) fn store_purchase_options(
    response: &OfferingsResponse,
    offerings: &Offerings,
) -> Vec<StoredPackagePurchaseOption> {
    let plans = response
        .offerings
        .iter()
        .flat_map(|offering| {
            offering.packages.iter().map(|package| {
                (
                    (offering.identifier.clone(), package.identifier.clone()),
                    package.platform_product_plan_identifier.clone(),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    offerings
        .all
        .iter()
        .flat_map(|(offering_identifier, offering)| {
            offering.packages.iter().filter_map(|package| {
                plans
                    .get(&(offering_identifier.clone(), package.identifier.clone()))
                    .map(|plan_identifier| {
                        StoredPackagePurchaseOption::new(
                            package,
                            plan_identifier
                                .as_deref()
                                .map(StorePurchaseOption::for_plan),
                        )
                    })
            })
        })
        .collect()
}

pub(super) fn store_purchase_option_for_package(
    options: &[StoredPackagePurchaseOption],
    package: &Package,
    was_issued_by_sdk: bool,
) -> Result<Option<StorePurchaseOption>> {
    // Directly constructed 1.x Package values have no provenance and remain
    // valid with or without a fetched Offering snapshot. An SDK-issued package
    // that is absent from the current snapshot must fail closed instead.
    let mut matching = options.iter().filter(|option| option.matches(package));
    let Some(first) = matching.next() else {
        return if was_issued_by_sdk {
            Err(Error::new(
                ErrorCode::ProductNotAvailableForPurchaseError,
                "The package belongs to an earlier identity or offering snapshot. Fetch offerings again before purchasing.",
            ))
        } else {
            Ok(None)
        };
    };
    if matching.any(|candidate| candidate.option != first.option) {
        return Err(Error::new(
            ErrorCode::ProductNotAvailableForPurchaseError,
            "The package's store plan changed after it was loaded. Fetch offerings again before purchasing.",
        ));
    }
    Ok(first.option.clone())
}

pub(super) fn store_product_queries(response: &OfferingsResponse) -> Vec<StoreProductQuery> {
    let mut product_and_plan = response
        .offerings
        .iter()
        .flat_map(|offering| {
            offering.packages.iter().map(|package| {
                (
                    package.platform_product_identifier.clone(),
                    package.platform_product_plan_identifier.clone(),
                )
            })
        })
        .collect::<Vec<_>>();
    product_and_plan.sort();
    product_and_plan.dedup();
    product_and_plan
        .into_iter()
        .map(|(product_id, plan_identifier)| match plan_identifier {
            Some(plan_identifier) => StoreProductQuery::for_plan(product_id, plan_identifier),
            None => StoreProductQuery::new(product_id),
        })
        .collect()
}
