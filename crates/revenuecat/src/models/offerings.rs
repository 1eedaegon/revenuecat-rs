//! Offerings, parsed from `GET /v1/subscribers/{id}/offerings` and resolved
//! against store products fetched by `platform_product_identifier` — the same
//! two-step join the official SDKs perform in their `OfferingParser`s.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::store_product::StoreProduct;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct OfferingsResponse {
    #[serde(default)]
    pub current_offering_id: Option<String>,
    #[serde(default)]
    pub offerings: Vec<OfferingResponse>,
    #[serde(default)]
    pub placements: Option<PlacementsResponse>,
    #[serde(default)]
    pub targeting: Option<TargetingResponse>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OfferingResponse {
    pub identifier: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub packages: Vec<PackageResponse>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageResponse {
    pub identifier: String,
    pub platform_product_identifier: String,
    #[serde(default)]
    pub platform_product_plan_identifier: Option<String>,
    #[serde(default)]
    pub web_checkout_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlacementsResponse {
    #[serde(default)]
    pub fallback_offering_id: Option<String>,
    #[serde(default)]
    pub offering_ids_by_placement: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetingResponse {
    pub rule_id: String,
    pub revision: i64,
}

// ---------------------------------------------------------------------------
// Public models
// ---------------------------------------------------------------------------

/// Mirrors `PackageType`: well-known `$rc_*` identifiers get a type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum PackageType {
    Lifetime,
    Annual,
    SixMonth,
    ThreeMonth,
    TwoMonth,
    Monthly,
    Weekly,
    Custom(String),
}

impl PackageType {
    pub fn from_identifier(identifier: &str) -> Self {
        match identifier {
            "$rc_lifetime" => Self::Lifetime,
            "$rc_annual" => Self::Annual,
            "$rc_six_month" => Self::SixMonth,
            "$rc_three_month" => Self::ThreeMonth,
            "$rc_two_month" => Self::TwoMonth,
            "$rc_monthly" => Self::Monthly,
            "$rc_weekly" => Self::Weekly,
            other => Self::Custom(other.to_owned()),
        }
    }
}

/// Where a purchase was initiated from; posted (flattened) with the receipt
/// so charts/experiments attribute revenue to the right offering.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PresentedOfferingContext {
    pub offering_identifier: Option<String>,
    pub placement_identifier: Option<String>,
    pub targeting: Option<TargetingResponse>,
}

/// Mirrors `Package`: a product wrapped in offering context.
#[derive(Debug, Clone, Serialize)]
pub struct Package {
    pub identifier: String,
    pub package_type: PackageType,
    pub store_product: StoreProduct,
    pub presented_offering_context: PresentedOfferingContext,
}

/// Mirrors `Offering`.
#[derive(Debug, Clone, Serialize)]
pub struct Offering {
    pub identifier: String,
    pub server_description: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub packages: Vec<Package>,
}

impl Offering {
    pub fn package(&self, identifier: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.identifier == identifier)
    }

    pub fn monthly(&self) -> Option<&Package> {
        self.packages
            .iter()
            .find(|p| p.package_type == PackageType::Monthly)
    }

    pub fn annual(&self) -> Option<&Package> {
        self.packages
            .iter()
            .find(|p| p.package_type == PackageType::Annual)
    }

    pub fn lifetime(&self) -> Option<&Package> {
        self.packages
            .iter()
            .find(|p| p.package_type == PackageType::Lifetime)
    }
}

/// Mirrors `Offerings`.
#[derive(Debug, Clone, Serialize)]
pub struct Offerings {
    pub all: BTreeMap<String, Offering>,
    pub current_offering_id: Option<String>,
}

impl Offerings {
    pub fn current(&self) -> Option<&Offering> {
        self.current_offering_id
            .as_ref()
            .and_then(|id| self.all.get(id))
    }

    pub fn offering(&self, identifier: &str) -> Option<&Offering> {
        self.all.get(identifier)
    }

    /// Joins the offerings response with fetched store products. Packages
    /// whose product is missing are dropped, and offerings with no resolvable
    /// packages are dropped, matching official `OfferingParser` behavior.
    pub fn resolve(
        response: &OfferingsResponse,
        products: &BTreeMap<String, StoreProduct>,
    ) -> Self {
        let targeting = response.targeting.clone();
        let all: BTreeMap<String, Offering> = response
            .offerings
            .iter()
            .filter_map(|offering| {
                let packages: Vec<Package> = offering
                    .packages
                    .iter()
                    .filter_map(|package| {
                        products
                            .get(&package.platform_product_identifier)
                            .map(|product| Package {
                                identifier: package.identifier.clone(),
                                package_type: PackageType::from_identifier(&package.identifier),
                                store_product: product.clone(),
                                presented_offering_context: PresentedOfferingContext {
                                    offering_identifier: Some(offering.identifier.clone()),
                                    placement_identifier: None,
                                    targeting: targeting.clone(),
                                },
                            })
                    })
                    .collect();
                if packages.is_empty() {
                    None
                } else {
                    Some((
                        offering.identifier.clone(),
                        Offering {
                            identifier: offering.identifier.clone(),
                            server_description: offering.description.clone(),
                            metadata: offering.metadata.clone(),
                            packages,
                        },
                    ))
                }
            })
            .collect();

        let current_offering_id = response
            .current_offering_id
            .clone()
            .filter(|id| all.contains_key(id));

        Self {
            all,
            current_offering_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::store_product::{Price, ProductType};

    const OFFERINGS_FIXTURE: &str = include_str!("../../tests/fixtures/offerings.json");

    fn product(id: &str) -> StoreProduct {
        StoreProduct {
            identifier: id.to_owned(),
            product_type: ProductType::Subscription,
            title: id.to_owned(),
            description: None,
            price: Price {
                amount_micros: 3_000_000,
                currency: "USD".into(),
            },
            subscription_period: Some("P1M".into()),
            trial: None,
            intro_price: None,
        }
    }

    #[test]
    fn parses_purchases_js_offerings_fixture() {
        let response: OfferingsResponse = serde_json::from_str(OFFERINGS_FIXTURE).unwrap();
        assert_eq!(response.current_offering_id.as_deref(), Some("offering_1"));
        assert_eq!(response.offerings.len(), 2);
        assert_eq!(response.targeting.as_ref().unwrap().revision, 123);
        assert_eq!(
            response
                .placements
                .as_ref()
                .unwrap()
                .offering_ids_by_placement["test_placement_id"],
            Some("offering_2".to_owned())
        );
    }

    #[test]
    fn resolves_packages_against_products() {
        // Arrange
        let response: OfferingsResponse = serde_json::from_str(OFFERINGS_FIXTURE).unwrap();
        let products = BTreeMap::from([("monthly".to_owned(), product("monthly"))]);

        // Act
        let offerings = Offerings::resolve(&response, &products);

        // Assert: offering_2 references product "monthly_2" which is missing,
        // so only offering_1 survives.
        assert_eq!(offerings.all.len(), 1);
        let current = offerings.current().unwrap();
        assert_eq!(current.identifier, "offering_1");
        assert_eq!(current.packages.len(), 1);
        assert_eq!(current.packages[0].package_type, PackageType::Monthly);
        assert_eq!(
            current.packages[0]
                .presented_offering_context
                .offering_identifier
                .as_deref(),
            Some("offering_1")
        );
    }

    #[test]
    fn current_offering_id_dropped_when_offering_missing() {
        let response: OfferingsResponse = serde_json::from_str(OFFERINGS_FIXTURE).unwrap();
        let products = BTreeMap::from([("monthly_2".to_owned(), product("monthly_2"))]);

        let offerings = Offerings::resolve(&response, &products);

        assert!(offerings.current().is_none());
        assert!(offerings.offering("offering_2").is_some());
    }

    #[test]
    fn package_type_from_identifier() {
        assert_eq!(
            PackageType::from_identifier("$rc_annual"),
            PackageType::Annual
        );
        assert_eq!(
            PackageType::from_identifier("my_pack"),
            PackageType::Custom("my_pack".to_owned())
        );
    }
}
