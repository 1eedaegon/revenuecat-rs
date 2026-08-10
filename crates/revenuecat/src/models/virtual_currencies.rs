//! Virtual currencies, parsed from
//! `GET /v1/subscribers/{id}/virtual_currencies` — the same shape
//! purchases-js's `VirtualCurrenciesResponse` documents.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualCurrency {
    pub balance: i64,
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VirtualCurrencies {
    #[serde(rename = "virtual_currencies", default)]
    pub all: BTreeMap<String, VirtualCurrency>,
}

impl VirtualCurrencies {
    pub fn get(&self, code: &str) -> Option<&VirtualCurrency> {
        self.all.get(code)
    }

    pub fn balance(&self, code: &str) -> i64 {
        self.get(code).map(|c| c.balance).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_virtual_currencies_response() {
        // Arrange: shape from purchases-js VirtualCurrenciesResponse.
        let json = r#"{
            "virtual_currencies": {
                "GLD": {"balance": 100, "code": "GLD", "name": "Gold"},
                "SLV": {"balance": -5, "code": "SLV", "name": "Silver", "description": "s"}
            }
        }"#;

        // Act
        let currencies: VirtualCurrencies = serde_json::from_str(json).unwrap();

        // Assert
        assert_eq!(currencies.balance("GLD"), 100);
        assert_eq!(
            currencies.balance("SLV"),
            -5,
            "negative balances appear in real fixtures"
        );
        assert_eq!(currencies.balance("UNKNOWN"), 0);
        assert_eq!(
            currencies.get("SLV").unwrap().description.as_deref(),
            Some("s")
        );
    }
}
