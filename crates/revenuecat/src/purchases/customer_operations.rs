use std::collections::BTreeMap;

use crate::diagnostics::MAX_EVENTS_PER_REQUEST;
use crate::error::{Error, ErrorCode, Result};
use crate::models::{RedeemResult, VirtualCurrencies, WebPurchaseRedemption};

use super::Purchases;

impl Purchases {
    /// Sets subscriber attributes; `None` values delete keys. Reserved keys
    /// use `$` prefixes (`$email`, `$displayName`, ...).
    pub async fn set_attributes(&self, attributes: BTreeMap<String, Option<String>>) -> Result<()> {
        if attributes.is_empty() {
            return Err(Error::new(
                ErrorCode::EmptySubscriberAttributesError,
                "Attributes are empty.",
            ));
        }
        let app_user_id = self.app_user_id_when_identity_settled().await;
        self.inner
            .backend
            .post_subscriber_attributes(&app_user_id, &attributes)
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

    /// Parses a RevenueCat redemption deep link
    /// (`<scheme>://redeem_web_purchase?redemption_token=...`), mirroring
    /// `Purchases.parseAsWebPurchaseRedemption`.
    pub fn parse_web_purchase_redemption(url: &str) -> Option<WebPurchaseRedemption> {
        WebPurchaseRedemption::parse_url(url)
    }

    /// Redeems a web purchase for the current user. On success the returned
    /// CustomerInfo is cached and listeners are notified, matching
    /// `WebPurchaseRedemptionHelper` in the official SDKs.
    pub async fn redeem_web_purchase(
        &self,
        redemption: &WebPurchaseRedemption,
    ) -> Result<RedeemResult> {
        let app_user_id = self.app_user_id_when_identity_settled().await;
        let result = self
            .inner
            .backend
            .post_redeem_web_purchase(&app_user_id, &redemption.redemption_token)
            .await?;
        if let RedeemResult::Success { customer_info } = &result {
            self.store_and_notify_for_user(&app_user_id, customer_info);
        }
        Ok(result)
    }

    /// Fetches virtual currency balances, serving a fresh cache when
    /// available — mirroring `getVirtualCurrencies`.
    pub async fn get_virtual_currencies(&self) -> Result<VirtualCurrencies> {
        let app_user_id = self.app_user_id_when_identity_settled().await;
        if let Some((currencies, false)) = self.inner.cache.cached_virtual_currencies(&app_user_id)
        {
            return Ok(currencies);
        }
        let currencies = self
            .inner
            .backend
            .get_virtual_currencies(&app_user_id)
            .await?;
        self.inner
            .cache
            .store_virtual_currencies(&app_user_id, &currencies);
        Ok(currencies)
    }

    /// Mirrors `cachedVirtualCurrencies`: the cache regardless of staleness.
    pub fn cached_virtual_currencies(&self) -> Option<VirtualCurrencies> {
        let app_user_id = self.app_user_id();
        self.inner
            .cache
            .cached_virtual_currencies(&app_user_id)
            .map(|(currencies, _)| currencies)
    }

    pub fn invalidate_virtual_currencies_cache(&self) {
        self.inner
            .cache
            .invalidate_virtual_currencies(&self.app_user_id());
    }

    /// Posts queued diagnostics entries in batches (200 per request) to the
    /// diagnostics host, following Android's retry/clear semantics. No-op
    /// when diagnostics is disabled or nothing is queued.
    pub async fn flush_diagnostics(&self) -> Result<()> {
        while let Some(batch) = self.inner.diagnostics.take_batch() {
            let result = self
                .inner
                .backend
                .post_diagnostics(&self.inner.config.diagnostics_url, &batch)
                .await;
            match result {
                Ok(()) => self.inner.diagnostics.batch_succeeded(),
                Err(error)
                    if matches!(
                        error.code,
                        ErrorCode::NetworkError | ErrorCode::UnknownBackendError
                    ) =>
                {
                    self.inner.diagnostics.batch_failed_retryable(batch);
                    return Err(error);
                }
                Err(error) => {
                    self.inner.diagnostics.batch_failed_fatal();
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Fire-and-forget flush once a full batch has accumulated, standing in
    /// for Android's 200 KB file-size trigger.
    pub(super) fn maybe_auto_flush(&self) {
        if self.inner.diagnostics.pending() >= MAX_EVENTS_PER_REQUEST {
            let purchases = self.clone();
            tauri_agnostic_spawn(async move {
                let _ = purchases.flush_diagnostics().await;
            });
        }
    }
}

/// Spawns on the ambient tokio runtime when one exists; otherwise drops the
/// task (auto-flush is best-effort — explicit flush_diagnostics remains).
fn tauri_agnostic_spawn<F: std::future::Future<Output = ()> + Send + 'static>(future: F) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(future);
    }
}
