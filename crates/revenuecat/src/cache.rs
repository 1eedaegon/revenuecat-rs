//! In-memory device cache, mirroring the caching responsibilities of
//! `DeviceCache` in the official SDKs (per-user CustomerInfo + offerings,
//! with a staleness window checked by fetch policies).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::models::{CustomerInfo, Offerings, VirtualCurrencies};

#[derive(Debug, Clone)]
struct CachedEntry<T> {
    value: T,
    stored_at: Instant,
}

#[derive(Debug)]
pub(crate) struct DeviceCache {
    ttl: Duration,
    customer_info: Mutex<HashMap<String, CachedEntry<CustomerInfo>>>,
    offerings: Mutex<Option<CachedEntry<Offerings>>>,
    virtual_currencies: Mutex<Option<CachedEntry<VirtualCurrencies>>>,
}

impl DeviceCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            customer_info: Mutex::new(HashMap::new()),
            offerings: Mutex::new(None),
            virtual_currencies: Mutex::new(None),
        }
    }

    /// Returns the cached info and whether it is stale (older than the TTL).
    pub fn cached_customer_info(&self, app_user_id: &str) -> Option<(CustomerInfo, bool)> {
        let guard = self.customer_info.lock().expect("cache lock poisoned");
        guard
            .get(app_user_id)
            .map(|entry| (entry.value.clone(), entry.stored_at.elapsed() > self.ttl))
    }

    pub fn store_customer_info(&self, app_user_id: &str, info: &CustomerInfo) {
        self.customer_info
            .lock()
            .expect("cache lock poisoned")
            .insert(
                app_user_id.to_owned(),
                CachedEntry {
                    value: info.clone(),
                    stored_at: Instant::now(),
                },
            );
    }

    pub fn invalidate_customer_info(&self, app_user_id: &str) {
        self.customer_info
            .lock()
            .expect("cache lock poisoned")
            .remove(app_user_id);
    }

    pub fn cached_offerings(&self) -> Option<(Offerings, bool)> {
        let guard = self.offerings.lock().expect("cache lock poisoned");
        guard
            .as_ref()
            .map(|entry| (entry.value.clone(), entry.stored_at.elapsed() > self.ttl))
    }

    pub fn store_offerings(&self, offerings: &Offerings) {
        *self.offerings.lock().expect("cache lock poisoned") = Some(CachedEntry {
            value: offerings.clone(),
            stored_at: Instant::now(),
        });
    }

    pub fn cached_virtual_currencies(&self) -> Option<(VirtualCurrencies, bool)> {
        let guard = self.virtual_currencies.lock().expect("cache lock poisoned");
        guard
            .as_ref()
            .map(|entry| (entry.value.clone(), entry.stored_at.elapsed() > self.ttl))
    }

    pub fn store_virtual_currencies(&self, currencies: &VirtualCurrencies) {
        *self.virtual_currencies.lock().expect("cache lock poisoned") = Some(CachedEntry {
            value: currencies.clone(),
            stored_at: Instant::now(),
        });
    }

    pub fn invalidate_virtual_currencies(&self) {
        *self.virtual_currencies.lock().expect("cache lock poisoned") = None;
    }

    /// Drops everything — used on identity changes (logIn/logOut).
    pub fn clear(&self) {
        self.customer_info
            .lock()
            .expect("cache lock poisoned")
            .clear();
        *self.offerings.lock().expect("cache lock poisoned") = None;
        *self.virtual_currencies.lock().expect("cache lock poisoned") = None;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::models::{VirtualCurrencies, VirtualCurrency};

    fn customer_info(user: &str) -> CustomerInfo {
        let raw = serde_json::json!({
            "request_date": "2026-01-01T00:00:00Z",
            "subscriber": {
                "original_app_user_id": user,
                "first_seen": "2026-01-01T00:00:00Z",
            }
        });
        CustomerInfo::from_response(raw).unwrap()
    }

    #[test]
    fn caches_customer_info_per_user() {
        // Arrange
        let cache = DeviceCache::new(Duration::from_secs(300));
        cache.store_customer_info("a", &customer_info("a"));

        // Act / Assert: user isolation.
        let (cached, stale) = cache.cached_customer_info("a").unwrap();
        assert_eq!(cached.original_app_user_id, "a");
        assert!(!stale, "fresh entry must not be stale");
        assert!(cache.cached_customer_info("b").is_none());
    }

    #[test]
    fn zero_ttl_marks_entries_stale_immediately() {
        let cache = DeviceCache::new(Duration::ZERO);
        cache.store_customer_info("a", &customer_info("a"));
        let (_, stale) = cache.cached_customer_info("a").unwrap();
        assert!(stale);
    }

    #[test]
    fn invalidate_removes_only_that_user() {
        let cache = DeviceCache::new(Duration::from_secs(300));
        cache.store_customer_info("a", &customer_info("a"));
        cache.store_customer_info("b", &customer_info("b"));

        cache.invalidate_customer_info("a");

        assert!(cache.cached_customer_info("a").is_none());
        assert!(cache.cached_customer_info("b").is_some());
    }

    #[test]
    fn clear_drops_all_caches() {
        // Arrange: every cache slot populated.
        let cache = DeviceCache::new(Duration::from_secs(300));
        cache.store_customer_info("a", &customer_info("a"));
        cache.store_offerings(&crate::models::Offerings {
            all: Default::default(),
            current_offering_id: None,
        });
        cache.store_virtual_currencies(&VirtualCurrencies {
            all: [(
                "GLD".to_owned(),
                VirtualCurrency {
                    balance: 1,
                    code: "GLD".into(),
                    name: "Gold".into(),
                    description: None,
                },
            )]
            .into(),
        });

        // Act
        cache.clear();

        // Assert
        assert!(cache.cached_customer_info("a").is_none());
        assert!(cache.cached_offerings().is_none());
        assert!(cache.cached_virtual_currencies().is_none());
    }

    #[test]
    fn virtual_currencies_report_staleness() {
        let cache = DeviceCache::new(Duration::ZERO);
        cache.store_virtual_currencies(&VirtualCurrencies::default());
        let (_, stale) = cache.cached_virtual_currencies().unwrap();
        assert!(stale);
    }
}
