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
