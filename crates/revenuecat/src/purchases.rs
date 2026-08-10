//! The `Purchases` facade — the public entry point, mirroring the `Purchases`
//! singleton of the official SDKs. Unlike them it is instance-based (idiomatic
//! for Rust and friendlier to tests); hold it in your app state and clone
//! freely (it is an `Arc` handle).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::backend::{Backend, ReceiptRequest};
use crate::billing::simulated::missing_store_billing_error;
use crate::billing::{SimulatedStoreBilling, StoreBilling};
use crate::cache::DeviceCache;
use crate::configuration::{ApiKeyKind, Configuration};
use crate::error::{Error, ErrorCode, Result};
use crate::http::HttpClient;
use crate::identity::IdentityManager;
use crate::models::{CustomerInfo, Offerings, Package, StoreProduct, StoreTransaction};

/// Mirrors `CacheFetchPolicy` from purchases-android.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheFetchPolicy {
    /// Cached value or error; never hits the network.
    CacheOnly,
    /// Always fetches from the backend.
    FetchCurrent,
    /// Cached value regardless of staleness, fetching only on a cache miss.
    CachedOrFetched,
    /// Cached value if fresh, otherwise fetch — the SDK default.
    #[default]
    NotStaleCachedOrFetched,
}

/// Result of a successful purchase.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PurchaseResult {
    pub transaction: StoreTransaction,
    pub customer_info: CustomerInfo,
}

type CustomerInfoListener = Box<dyn Fn(&CustomerInfo) + Send + Sync>;

struct PurchasesInner {
    config: Configuration,
    identity: Arc<IdentityManager>,
    backend: Arc<Backend>,
    billing: Arc<dyn StoreBilling>,
    cache: DeviceCache,
    listener: Mutex<Option<CustomerInfoListener>>,
}

/// Cheaply cloneable handle to a configured SDK instance.
#[derive(Clone)]
pub struct Purchases {
    inner: Arc<PurchasesInner>,
}

impl Purchases {
    /// Configures a new SDK instance. With a `test_` API key the built-in
    /// simulated Test Store is used; other keys require a
    /// [`StoreBilling`] implementation via the configuration builder.
    pub fn configure(config: Configuration) -> Result<Self> {
        let http = HttpClient::new(&config)?;
        let backend = Arc::new(Backend::new(http));
        let identity = Arc::new(IdentityManager::new(config.app_user_id.clone()));

        let billing: Arc<dyn StoreBilling> = match &config.store_billing {
            Some(billing) => Arc::clone(billing),
            None if config.api_key_kind() == ApiKeyKind::TestStore => Arc::new(
                SimulatedStoreBilling::new(Arc::clone(&backend), Arc::clone(&identity)),
            ),
            None => return Err(missing_store_billing_error()),
        };

        let cache = DeviceCache::new(config.cache_ttl);
        Ok(Self {
            inner: Arc::new(PurchasesInner {
                config,
                identity,
                backend,
                billing,
                cache,
                listener: Mutex::new(None),
            }),
        })
    }

    pub fn app_user_id(&self) -> String {
        self.inner.identity.current_app_user_id()
    }

    pub fn is_anonymous(&self) -> bool {
        self.inner.identity.is_anonymous()
    }

    /// Registers a listener invoked whenever fresh CustomerInfo arrives,
    /// mirroring `updatedCustomerInfoListener`.
    pub fn set_customer_info_listener(
        &self,
        listener: impl Fn(&CustomerInfo) + Send + Sync + 'static,
    ) {
        *self.inner.listener.lock().expect("listener lock poisoned") = Some(Box::new(listener));
    }

    // -- Customer info -----------------------------------------------------

    pub async fn get_customer_info(&self, policy: CacheFetchPolicy) -> Result<CustomerInfo> {
        let app_user_id = self.app_user_id();
        let cached = self.inner.cache.cached_customer_info(&app_user_id);
        match policy {
            CacheFetchPolicy::CacheOnly => cached.map(|(info, _)| info).ok_or_else(|| {
                Error::new(
                    ErrorCode::CustomerInfoError,
                    "No cached CustomerInfo available.",
                )
            }),
            CacheFetchPolicy::FetchCurrent => {
                self.fetch_and_cache_customer_info(&app_user_id).await
            }
            CacheFetchPolicy::CachedOrFetched => match cached {
                Some((info, _)) => Ok(info),
                None => self.fetch_and_cache_customer_info(&app_user_id).await,
            },
            CacheFetchPolicy::NotStaleCachedOrFetched => match cached {
                Some((info, false)) => Ok(info),
                _ => self.fetch_and_cache_customer_info(&app_user_id).await,
            },
        }
    }

    pub fn invalidate_customer_info_cache(&self) {
        self.inner
            .cache
            .invalidate_customer_info(&self.app_user_id());
    }

    // -- Offerings & products ---------------------------------------------

    /// Fetches offerings and resolves each package against store products —
    /// the same backend-JSON + store-product join the official SDKs do.
    pub async fn get_offerings(&self) -> Result<Offerings> {
        if let Some((offerings, false)) = self.inner.cache.cached_offerings() {
            return Ok(offerings);
        }
        let app_user_id = self.app_user_id();
        let response = self.inner.backend.get_offerings(&app_user_id).await?;

        let product_ids: Vec<String> = {
            let mut ids: Vec<String> = response
                .offerings
                .iter()
                .flat_map(|o| {
                    o.packages
                        .iter()
                        .map(|p| p.platform_product_identifier.clone())
                })
                .collect();
            ids.sort();
            ids.dedup();
            ids
        };
        let products = self.inner.billing.query_products(&product_ids).await?;
        let by_id: BTreeMap<String, StoreProduct> = products
            .into_iter()
            .map(|p| (p.identifier.clone(), p))
            .collect();

        let offerings = Offerings::resolve(&response, &by_id);
        self.inner.cache.store_offerings(&offerings);
        Ok(offerings)
    }

    /// Fetches store products directly by identifier (`getProducts`).
    pub async fn get_products(&self, product_ids: &[String]) -> Result<Vec<StoreProduct>> {
        self.inner.billing.query_products(product_ids).await
    }

    // -- Purchasing --------------------------------------------------------

    /// Purchases a package from an offering (carries offering context for
    /// revenue attribution).
    pub async fn purchase_package(&self, package: &Package) -> Result<PurchaseResult> {
        self.purchase_internal(
            &package.store_product,
            package
                .presented_offering_context
                .offering_identifier
                .clone(),
            package
                .presented_offering_context
                .placement_identifier
                .clone(),
        )
        .await
    }

    /// Purchases a bare product (no offering context).
    pub async fn purchase_product(&self, product: &StoreProduct) -> Result<PurchaseResult> {
        self.purchase_internal(product, None, None).await
    }

    async fn purchase_internal(
        &self,
        product: &StoreProduct,
        offering_identifier: Option<String>,
        placement_identifier: Option<String>,
    ) -> Result<PurchaseResult> {
        let transaction = self.inner.billing.purchase(product).await?;

        let request = ReceiptRequest {
            fetch_token: transaction.purchase_token.clone(),
            app_user_id: self.app_user_id(),
            product_ids: transaction.product_ids.clone(),
            is_restore: false,
            observer_mode: false,
            initiation_source: "purchase",
            price: Some(product.price.amount_micros as f64 / 1_000_000.0),
            currency: Some(product.price.currency.clone()),
            normal_duration: product.subscription_period.clone(),
            presented_offering_identifier: offering_identifier,
            presented_placement_identifier: placement_identifier,
            applied_targeting_rule: None,
        };

        let customer_info = self.inner.backend.post_receipt(&request).await?;
        // Finish only after the backend accepted the receipt — official SDK
        // ordering that prevents losing purchases.
        self.inner
            .billing
            .finish_transaction(&transaction, true)
            .await?;
        self.store_and_notify(&customer_info);
        Ok(PurchaseResult {
            transaction,
            customer_info,
        })
    }

    /// Posts currently-owned store transactions as restores; with no owned
    /// transactions it simply refreshes CustomerInfo (Android behavior).
    pub async fn restore_purchases(&self) -> Result<CustomerInfo> {
        let app_user_id = self.app_user_id();
        let transactions = self.inner.billing.query_purchases().await?;
        if transactions.is_empty() {
            return self.fetch_and_cache_customer_info(&app_user_id).await;
        }

        let mut latest: Option<CustomerInfo> = None;
        for transaction in &transactions {
            let request = ReceiptRequest {
                fetch_token: transaction.purchase_token.clone(),
                app_user_id: app_user_id.clone(),
                product_ids: transaction.product_ids.clone(),
                is_restore: true,
                observer_mode: false,
                initiation_source: "restore",
                price: transaction
                    .price
                    .as_ref()
                    .map(|p| p.amount_micros as f64 / 1_000_000.0),
                currency: transaction.price.as_ref().map(|p| p.currency.clone()),
                normal_duration: None,
                presented_offering_identifier: None,
                presented_placement_identifier: None,
                applied_targeting_rule: None,
            };
            let info = self.inner.backend.post_receipt(&request).await?;
            self.inner
                .billing
                .finish_transaction(transaction, false)
                .await?;
            latest = Some(info);
        }
        let info = latest.ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownError,
                "Restore produced no customer info.",
            )
        })?;
        self.store_and_notify(&info);
        Ok(info)
    }

    // -- Identity ----------------------------------------------------------

    /// Switches to `new_app_user_id`, aliasing the current user. Returns the
    /// new user's CustomerInfo and whether the user was just created.
    pub async fn log_in(&self, new_app_user_id: &str) -> Result<(CustomerInfo, bool)> {
        let current = self.app_user_id();
        if current == new_app_user_id {
            return Ok((
                self.get_customer_info(CacheFetchPolicy::CachedOrFetched)
                    .await?,
                false,
            ));
        }
        let (info, created) = self.inner.backend.log_in(&current, new_app_user_id).await?;
        self.inner.identity.switch_user(new_app_user_id);
        self.inner.cache.clear();
        self.store_and_notify(&info);
        Ok((info, created))
    }

    /// Resets to a new anonymous user. Errors if already anonymous, matching
    /// `LogOutWithAnonymousUserError` in the official SDKs.
    pub async fn log_out(&self) -> Result<CustomerInfo> {
        if self.is_anonymous() {
            return Err(Error::new(
                ErrorCode::LogOutWithAnonymousUserError,
                "Called logOut but the current user is anonymous.",
            ));
        }
        let new_anonymous = self.inner.identity.reset_to_anonymous();
        self.inner.cache.clear();
        self.fetch_and_cache_customer_info(&new_anonymous).await
    }

    // -- Subscriber attributes --------------------------------------------

    /// Sets subscriber attributes; `None` values delete keys. Reserved keys
    /// use `$` prefixes (`$email`, `$displayName`, ...).
    pub async fn set_attributes(&self, attributes: BTreeMap<String, Option<String>>) -> Result<()> {
        if attributes.is_empty() {
            return Err(Error::new(
                ErrorCode::EmptySubscriberAttributesError,
                "Attributes are empty.",
            ));
        }
        self.inner
            .backend
            .post_subscriber_attributes(&self.app_user_id(), &attributes)
            .await
    }

    pub async fn set_email(&self, email: impl Into<String>) -> Result<()> {
        self.set_attributes(BTreeMap::from([("$email".to_owned(), Some(email.into()))]))
            .await
    }

    pub async fn set_display_name(&self, name: impl Into<String>) -> Result<()> {
        self.set_attributes(BTreeMap::from([(
            "$displayName".to_owned(),
            Some(name.into()),
        )]))
        .await
    }

    // -- Internals ---------------------------------------------------------

    async fn fetch_and_cache_customer_info(&self, app_user_id: &str) -> Result<CustomerInfo> {
        let info = self.inner.backend.get_customer_info(app_user_id).await?;
        self.store_and_notify(&info);
        Ok(info)
    }

    fn store_and_notify(&self, info: &CustomerInfo) {
        self.inner
            .cache
            .store_customer_info(&self.app_user_id(), info);
        if let Some(listener) = &*self.inner.listener.lock().expect("listener lock poisoned") {
            listener(info);
        }
    }
}

impl std::fmt::Debug for Purchases {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Purchases")
            .field("app_user_id", &self.app_user_id())
            .field("config", &self.inner.config)
            .finish()
    }
}
