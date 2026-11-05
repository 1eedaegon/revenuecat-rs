//! The `Purchases` facade — the public entry point, mirroring the `Purchases`
//! singleton of the official SDKs. Unlike them it is instance-based (idiomatic
//! for Rust and friendlier to tests); hold it in your app state and clone
//! freely (it is an `Arc` handle).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::backend::{Backend, ReceiptRequest};
use crate::billing::simulated::missing_store_billing_error;
use crate::billing::{SimulatedStoreBilling, StoreBilling};
use crate::cache::DeviceCache;
use crate::configuration::{ApiKeyKind, Configuration};
use crate::diagnostics::DiagnosticsTracker;
use crate::error::{Error, ErrorCode, Result};
use crate::http::HttpClient;
use crate::identity::IdentityManager;
use crate::models::{
    CustomerInfo, Offerings, Package, ProductType, Store, StoreProduct, StorePurchaseOption,
    StoreReceiptContext, StoreTransaction, TargetingResponse,
};

mod customer_operations;
mod identity_operations;
mod store_options;

use store_options::{
    billed_base_plan_of_earlier_launch, store_product_queries, store_purchase_options,
    StorePurchaseOptionRegistry,
};

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

#[derive(Debug, Clone, Copy)]
enum StorePurchaseSyncMode {
    ExplicitRestore,
    FullQueue,
    PendingQueue,
}

impl StorePurchaseSyncMode {
    fn is_restore(self, is_anonymous: bool) -> bool {
        match self {
            Self::ExplicitRestore => true,
            Self::FullQueue | Self::PendingQueue => is_anonymous,
        }
    }

    fn initiation_source(self, store: Store) -> &'static str {
        match self {
            Self::ExplicitRestore => "restore",
            Self::FullQueue | Self::PendingQueue
                if matches!(store, Store::AppStore | Store::MacAppStore) =>
            {
                "queue"
            }
            Self::FullQueue | Self::PendingQueue => "unsynced_active_purchases",
        }
    }
}

struct PurchasesInner {
    config: Configuration,
    identity: Arc<IdentityManager>,
    backend: Arc<Backend>,
    billing: Arc<dyn StoreBilling>,
    cache: DeviceCache,
    diagnostics: Arc<DiagnosticsTracker>,
    listener: Mutex<Option<CustomerInfoListener>>,
    /// Serializes log-in/log-out so every identity request observes the
    /// identity produced by the previous operation.
    identity_operation: tokio::sync::Mutex<()>,
    /// Prevents explicit purchases, restores, and native queue reconciliation
    /// from posting/finishing the same transaction concurrently.
    store_transaction_operation: tokio::sync::Mutex<()>,
    /// Base-plan selections live outside public `Package` so 1.x package
    /// struct literals remain source compatible.
    store_purchase_options: Mutex<StorePurchaseOptionRegistry>,
    /// Base plans launched in this process, keyed by app user then product.
    /// Used with the backend-confirmed order/plan to reject a delayed Play
    /// transaction that Play stamped with a newer launch's option.
    launched_store_plans: Mutex<BTreeMap<String, BTreeMap<String, BTreeSet<String>>>>,
    /// A durable identity transition was prepared but has not been committed.
    /// Receipt-bearing operations fail closed until the transition is retried.
    identity_transition_pending: AtomicBool,
    /// Exact transition paired with `identity_transition_pending`. Keeping
    /// the kind and target under the identity-operation lock prevents a
    /// queued, different identity request from replacing the durable journal.
    pending_identity_transition: Mutex<Option<PendingIdentityTransition>>,
    /// A native purchase returned a transaction but backend posting or store
    /// finishing has not completed. A pending/full sync must recover it before
    /// another explicit purchase can open store UI.
    native_purchase_recovery_required: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentityTransitionKind {
    LogIn,
    LogOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingIdentityTransition {
    kind: IdentityTransitionKind,
    target: String,
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
        let diagnostics = Arc::new(DiagnosticsTracker::new(config.diagnostics_enabled));
        let http = HttpClient::new(&config, Arc::clone(&diagnostics))?;
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
                diagnostics,
                listener: Mutex::new(None),
                identity_operation: tokio::sync::Mutex::new(()),
                store_transaction_operation: tokio::sync::Mutex::new(()),
                store_purchase_options: Mutex::new(StorePurchaseOptionRegistry::default()),
                launched_store_plans: Mutex::new(BTreeMap::new()),
                identity_transition_pending: AtomicBool::new(false),
                pending_identity_transition: Mutex::new(None),
                native_purchase_recovery_required: AtomicBool::new(false),
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
        let app_user_id = self.app_user_id_when_identity_settled().await;
        self.get_customer_info_for_user(&app_user_id, policy).await
    }

    async fn get_customer_info_for_user(
        &self,
        app_user_id: &str,
        policy: CacheFetchPolicy,
    ) -> Result<CustomerInfo> {
        let cached = self.inner.cache.cached_customer_info(app_user_id);
        match policy {
            CacheFetchPolicy::CacheOnly => cached.map(|(info, _)| info).ok_or_else(|| {
                Error::new(
                    ErrorCode::CustomerInfoError,
                    "No cached CustomerInfo available.",
                )
            }),
            CacheFetchPolicy::FetchCurrent => self.fetch_and_cache_customer_info(app_user_id).await,
            CacheFetchPolicy::CachedOrFetched => match cached {
                Some((info, _)) => Ok(info),
                None => self.fetch_and_cache_customer_info(app_user_id).await,
            },
            CacheFetchPolicy::NotStaleCachedOrFetched => match cached {
                Some((info, false)) => Ok(info),
                _ => self.fetch_and_cache_customer_info(app_user_id).await,
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
        let app_user_id = self.app_user_id_when_identity_settled().await;
        if let Some((offerings, false)) = self.inner.cache.cached_offerings(&app_user_id) {
            return Ok(offerings);
        }
        let response = self.inner.backend.get_offerings(&app_user_id).await?;

        let queries = store_product_queries(&response);
        let products = self
            .inner
            .billing
            .query_products_with_options(&queries)
            .await?;
        let by_product_and_plan = products
            .into_iter()
            .map(|result| {
                let key = (
                    result.product().identifier.clone(),
                    result.platform_product_plan_identifier().map(str::to_owned),
                );
                (key, result.into_product())
            })
            .collect::<BTreeMap<_, _>>();

        let offerings = Offerings::resolve_with(&response, |package| {
            let exact_key = (
                package.platform_product_identifier.clone(),
                package.platform_product_plan_identifier.clone(),
            );
            let fallback_key = (package.platform_product_identifier.clone(), None);
            by_product_and_plan
                .get(&exact_key)
                .or_else(|| by_product_and_plan.get(&fallback_key))
                .cloned()
        });
        let purchase_options = store_purchase_options(&response, &offerings);
        // Never return a package loaded for an identity that changed while
        // backend/store queries were in flight. Otherwise the caller could
        // immediately purchase it under the new identity without its plan.
        let _identity_operation = self.inner.identity_operation.lock().await;
        if self.app_user_id() != app_user_id {
            return Err(Error::new(
                ErrorCode::OperationAlreadyInProgressError,
                "The App User ID changed while offerings were loading. Fetch offerings again before purchasing.",
            ));
        }
        self.inner
            .store_purchase_options
            .lock()
            .expect("store purchase options lock poisoned")
            .remember_for_user(&app_user_id, purchase_options);
        self.inner.cache.store_offerings(&app_user_id, &offerings);
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
        let _store_transaction_operation = self
            .inner
            .store_transaction_operation
            .try_lock()
            .map_err(|_| purchase_operation_in_progress_error())?;
        // Keep the initiating identity stable while native purchase UI is
        // visible and until its receipt has been posted and finished. A
        // concurrent logIn/logOut must never attribute user A's transaction
        // to user B.
        let _identity_operation = self.inner.identity_operation.lock().await;
        self.ensure_receipt_operations_ready()?;
        let app_user_id = self.app_user_id();
        let store_purchase_option = {
            self.inner
                .store_purchase_options
                .lock()
                .expect("store purchase options lock poisoned")
                .purchase_option_for_package(&app_user_id, package)?
        };
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
            package.presented_offering_context.targeting.clone(),
            store_purchase_option.as_ref(),
            &app_user_id,
        )
        .await
    }

    /// Purchases a bare product (no offering context).
    pub async fn purchase_product(&self, product: &StoreProduct) -> Result<PurchaseResult> {
        let _store_transaction_operation = self
            .inner
            .store_transaction_operation
            .try_lock()
            .map_err(|_| purchase_operation_in_progress_error())?;
        let _identity_operation = self.inner.identity_operation.lock().await;
        self.ensure_receipt_operations_ready()?;
        let app_user_id = self.app_user_id();
        self.purchase_internal(product, None, None, None, None, &app_user_id)
            .await
    }

    async fn purchase_internal(
        &self,
        product: &StoreProduct,
        offering_identifier: Option<String>,
        placement_identifier: Option<String>,
        targeting: Option<TargetingResponse>,
        store_purchase_option: Option<&StorePurchaseOption>,
        app_user_id: &str,
    ) -> Result<PurchaseResult> {
        validate_purchase_input(
            product,
            offering_identifier.as_deref(),
            placement_identifier.as_deref(),
            targeting.as_ref(),
            store_purchase_option,
        )?;
        let receipt_context = StoreReceiptContext::new(
            offering_identifier.clone(),
            placement_identifier.clone(),
            targeting.clone(),
            Some(product.price.clone()),
            product.subscription_period.clone(),
            true,
        );
        let launched_plan_id =
            store_purchase_option.and_then(StorePurchaseOption::platform_product_plan_identifier);
        if let Some(plan_id) = launched_plan_id {
            self.inner
                .launched_store_plans
                .lock()
                .expect("launched store plans lock poisoned")
                .entry(app_user_id.to_owned())
                .or_default()
                .entry(product.identifier.clone())
                .or_default()
                .insert(plan_id.to_owned());
        }
        // A native store can create a transaction and then fail while
        // persisting its local receipt journal or returning the IPC payload.
        // Arm recovery before crossing that boundary so Rust-owned adapters
        // cannot switch identity or reopen store UI after an ambiguous error.
        self.inner
            .native_purchase_recovery_required
            .store(true, Ordering::Release);
        let store_purchase = self
            .inner
            .billing
            .purchase_with_receipt_context_for_user(
                product,
                store_purchase_option,
                app_user_id,
                &receipt_context,
            )
            .await?;
        let transaction = store_purchase.transaction;

        let request = ReceiptRequest {
            fetch_token: transaction.purchase_token.clone(),
            app_user_id: app_user_id.to_owned(),
            product_ids: transaction.product_ids.clone(),
            is_restore: false,
            observer_mode: false,
            initiation_source: "purchase",
            price: Some(product.price.amount_micros as f64 / 1_000_000.0),
            currency: Some(product.price.currency.clone()),
            normal_duration: product.subscription_period.clone(),
            presented_offering_identifier: offering_identifier,
            presented_placement_identifier: placement_identifier,
            applied_targeting_rule: targeting,
        };

        let receipt_result = self
            .inner
            .backend
            .post_receipt(&request, &store_purchase.platform_product_ids)
            .await?;
        let mismatched_billed_plan = {
            let launched_plans = self
                .inner
                .launched_store_plans
                .lock()
                .expect("launched store plans lock poisoned");
            let plans_for_product = transaction.product_ids.first().and_then(|product_id| {
                launched_plans
                    .get(app_user_id)
                    .and_then(|products| products.get(product_id))
            });
            billed_base_plan_of_earlier_launch(
                &transaction,
                &receipt_result.customer_info,
                launched_plan_id,
                plans_for_product,
            )
        };
        // Finish only after the backend accepted the receipt — official SDK
        // ordering that prevents losing purchases. Only consumables are
        // consumed; consuming a subscription token is a Play Billing
        // developer error (subscriptions/entitlements get acknowledged).
        self.inner
            .billing
            .finish_transaction(
                &transaction,
                receipt_result
                    .should_consume
                    .unwrap_or(matches!(product.product_type, ProductType::Consumable)),
            )
            .await?;
        let customer_info = receipt_result.customer_info;
        self.store_and_notify_for_user(app_user_id, &customer_info);
        self.inner
            .native_purchase_recovery_required
            .store(false, Ordering::Release);
        if let Some(billed_plan_id) = mismatched_billed_plan {
            return Err(Error::new(
                ErrorCode::ProductAlreadyPurchasedError,
                format!(
                    "Google Play completed an earlier '{billed_plan_id}' base plan instead of the currently launched base plan. CustomerInfo was refreshed from the authoritative receipt; retry the intended purchase if it is still needed."
                ),
            ));
        }
        Ok(PurchaseResult {
            transaction,
            customer_info,
        })
    }

    /// Posts currently-owned store transactions as restores; with no owned
    /// transactions it simply refreshes CustomerInfo (Android behavior).
    pub async fn restore_purchases(&self) -> Result<CustomerInfo> {
        self.sync_store_purchases(StorePurchaseSyncMode::ExplicitRestore)
            .await
    }

    /// Reconciles transactions delivered by the native store queue.
    ///
    /// Unlike [`Purchases::restore_purchases`], this does not opt an
    /// identified user into store-account sharing semantics. Anonymous users
    /// retain the official SDK default (`is_restore = true`) so their first
    /// launch can recover purchases from the device store account.
    pub async fn sync_pending_purchases(&self) -> Result<CustomerInfo> {
        self.sync_store_purchases(StorePurchaseSyncMode::PendingQueue)
            .await
    }

    /// Reconciles every currently-owned store transaction without treating an
    /// identified user's call as an explicit restore. This is the startup /
    /// foreground recovery path: it recovers acknowledged or already-finished
    /// purchases when the host has not persisted an anonymous user ID or a
    /// sent-token cache.
    pub async fn sync_purchases(&self) -> Result<CustomerInfo> {
        self.sync_store_purchases(StorePurchaseSyncMode::FullQueue)
            .await
    }

    async fn sync_store_purchases(&self, mode: StorePurchaseSyncMode) -> Result<CustomerInfo> {
        // Query and post under one identity operation. Otherwise logIn/logOut
        // could alias/reset the captured user between store discovery and the
        // backend receipt post.
        let _store_transaction_operation = self.inner.store_transaction_operation.lock().await;
        let _identity_operation = self.inner.identity_operation.lock().await;
        if self
            .inner
            .identity_transition_pending
            .load(Ordering::Acquire)
        {
            return Err(identity_transition_pending_error());
        }
        if !matches!(mode, StorePurchaseSyncMode::ExplicitRestore) {
            // A native queue event may exist even when the query itself fails.
            // Bind all subsequent retries to this identity until a successful
            // empty query or complete reconciliation clears the flag.
            self.inner
                .native_purchase_recovery_required
                .store(true, Ordering::Release);
        }
        let store_purchases = match mode {
            StorePurchaseSyncMode::ExplicitRestore | StorePurchaseSyncMode::FullQueue => {
                self.inner.billing.query_purchases_with_context().await?
            }
            StorePurchaseSyncMode::PendingQueue => {
                self.inner
                    .billing
                    .query_pending_purchases_with_context()
                    .await?
            }
        };
        let app_user_id = self.app_user_id();
        let is_restore = mode.is_restore(self.is_anonymous());
        if store_purchases.is_empty() {
            let info = self.fetch_and_cache_customer_info(&app_user_id).await?;
            self.inner
                .native_purchase_recovery_required
                .store(false, Ordering::Release);
            return Ok(info);
        }
        self.inner
            .native_purchase_recovery_required
            .store(true, Ordering::Release);

        let mut latest: Option<CustomerInfo> = None;
        for store_purchase in &store_purchases {
            let transaction = &store_purchase.transaction;
            let receipt_context = store_purchase.receipt_context();
            let sdk_originated = receipt_context.is_some_and(StoreReceiptContext::sdk_originated);
            let recovered_price = transaction
                .price
                .as_ref()
                .or_else(|| receipt_context.and_then(StoreReceiptContext::price));
            let request = ReceiptRequest {
                fetch_token: transaction.purchase_token.clone(),
                app_user_id: app_user_id.clone(),
                product_ids: transaction.product_ids.clone(),
                is_restore: if sdk_originated { false } else { is_restore },
                observer_mode: false,
                initiation_source: if sdk_originated {
                    "purchase"
                } else {
                    mode.initiation_source(transaction.store)
                },
                price: recovered_price.map(|p| p.amount_micros as f64 / 1_000_000.0),
                currency: recovered_price.map(|p| p.currency.clone()),
                normal_duration: receipt_context
                    .and_then(StoreReceiptContext::normal_duration)
                    .map(str::to_owned),
                presented_offering_identifier: receipt_context
                    .and_then(StoreReceiptContext::presented_offering_identifier)
                    .map(str::to_owned),
                presented_placement_identifier: receipt_context
                    .and_then(StoreReceiptContext::presented_placement_identifier)
                    .map(str::to_owned),
                applied_targeting_rule: receipt_context
                    .and_then(StoreReceiptContext::applied_targeting_rule)
                    .cloned(),
            };
            let receipt_result = self
                .inner
                .backend
                .post_receipt(&request, &store_purchase.platform_product_ids)
                .await?;
            self.inner
                .billing
                .finish_transaction(transaction, receipt_result.should_consume.unwrap_or(false))
                .await?;
            let customer_info = receipt_result.customer_info;
            self.store_and_notify_for_user(&app_user_id, &customer_info);
            latest = Some(customer_info);
        }
        let info = latest.ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownError,
                "Restore produced no customer info.",
            )
        })?;
        self.inner
            .native_purchase_recovery_required
            .store(false, Ordering::Release);
        Ok(info)
    }

    // -- Internals ---------------------------------------------------------

    async fn app_user_id_when_identity_settled(&self) -> String {
        let _identity_operation = self.inner.identity_operation.lock().await;
        self.app_user_id()
    }

    async fn fetch_and_cache_customer_info(&self, app_user_id: &str) -> Result<CustomerInfo> {
        let info = self.inner.backend.get_customer_info(app_user_id).await?;
        self.store_and_notify_for_user(app_user_id, &info);
        Ok(info)
    }

    fn store_and_notify_for_user(&self, app_user_id: &str, info: &CustomerInfo) {
        // `customer_info_verification_result` is skipped for NotRequested,
        // matching trackCustomerInfoVerificationResultIfNeeded.
        if info.entitlements.verification != crate::models::VerificationResult::NotRequested {
            self.inner.diagnostics.track(
                "customer_info_verification_result",
                serde_json::json!({
                    "verification_result": format!("{:?}", info.entitlements.verification)
                }),
            );
        }
        self.maybe_auto_flush();
        self.inner.cache.store_customer_info(app_user_id, info);
        if self.app_user_id() == app_user_id {
            if let Some(listener) = &*self.inner.listener.lock().expect("listener lock poisoned") {
                listener(info);
            }
        }
    }

    fn ensure_receipt_operations_ready(&self) -> Result<()> {
        if self
            .inner
            .identity_transition_pending
            .load(Ordering::Acquire)
        {
            return Err(identity_transition_pending_error());
        }
        if self
            .inner
            .native_purchase_recovery_required
            .load(Ordering::Acquire)
        {
            return Err(native_purchase_recovery_error());
        }
        Ok(())
    }
}

fn validate_purchase_input(
    product: &StoreProduct,
    offering_identifier: Option<&str>,
    placement_identifier: Option<&str>,
    targeting: Option<&TargetingResponse>,
    store_purchase_option: Option<&StorePurchaseOption>,
) -> Result<()> {
    if product.price.amount_micros < 0 {
        return Err(invalid_purchase_input_error("product price"));
    }
    let text_fields = [
        ("product identifier", Some(product.identifier.as_str())),
        ("currency", Some(product.price.currency.as_str())),
        (
            "subscription period",
            product.subscription_period.as_deref(),
        ),
        ("offering identifier", offering_identifier),
        ("placement identifier", placement_identifier),
        (
            "targeting rule identifier",
            targeting.map(|value| value.rule_id.as_str()),
        ),
        (
            "platform product plan identifier",
            store_purchase_option.and_then(StorePurchaseOption::platform_product_plan_identifier),
        ),
    ];
    if let Some((field, _)) = text_fields
        .into_iter()
        .find(|(_, value)| value.is_some_and(|value| value.trim().is_empty()))
    {
        return Err(invalid_purchase_input_error(field));
    }
    Ok(())
}

fn invalid_purchase_input_error(field: &str) -> Error {
    Error::new(
        ErrorCode::ConfigurationError,
        format!("Cannot start a store purchase with an invalid {field}."),
    )
}

fn purchase_operation_in_progress_error() -> Error {
    Error::new(
        ErrorCode::OperationAlreadyInProgressError,
        "Another purchase or store transaction synchronization is already in progress.",
    )
}

fn identity_transition_pending_error() -> Error {
    Error::new(
        ErrorCode::OperationAlreadyInProgressError,
        "A durable App User ID transition is incomplete. Retry logIn/logOut before performing receipt operations.",
    )
}

fn native_purchase_recovery_error() -> Error {
    Error::new(
        ErrorCode::OperationAlreadyInProgressError,
        "A previous native purchase still requires synchronization. Call sync_pending_purchases before purchasing or changing App User ID.",
    )
}

impl std::fmt::Debug for Purchases {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Purchases")
            .field("app_user_id", &"[REDACTED]")
            .field("config", &self.inner.config)
            .finish()
    }
}

#[cfg(test)]
mod tests;
