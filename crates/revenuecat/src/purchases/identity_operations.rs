use std::sync::atomic::Ordering;

use crate::error::{Error, ErrorCode, Result};
use crate::models::CustomerInfo;

use super::{
    native_purchase_recovery_error, CacheFetchPolicy, IdentityTransitionKind,
    PendingIdentityTransition, Purchases,
};

impl Purchases {
    /// Switches to `new_app_user_id`, aliasing the current user. Returns the
    /// new user's CustomerInfo and whether the user was just created.
    pub async fn log_in(&self, new_app_user_id: &str) -> Result<(CustomerInfo, bool)> {
        self.log_in_internal(new_app_user_id, false, |_, _| Ok(()), |_| Ok(()))
            .await
    }

    /// Runs an identity commit after the backend accepts the login but before
    /// the in-memory identity changes.
    ///
    /// The prepare callback runs before the backend request, allowing a
    /// durable transition journal to be written. A commit failure leaves the
    /// current identity unchanged and blocks receipt-bearing operations until
    /// the same transition is resumed.
    #[doc(hidden)]
    pub async fn log_in_with_identity_transaction<P, C>(
        &self,
        new_app_user_id: &str,
        prepare: P,
        commit: C,
    ) -> Result<(CustomerInfo, bool)>
    where
        P: FnOnce(&str, &str) -> Result<()>,
        C: FnOnce(&str) -> Result<()>,
    {
        self.log_in_internal(new_app_user_id, true, prepare, commit)
            .await
    }

    async fn log_in_internal<P, C>(
        &self,
        new_app_user_id: &str,
        tracks_durable_transition: bool,
        prepare: P,
        commit: C,
    ) -> Result<(CustomerInfo, bool)>
    where
        P: FnOnce(&str, &str) -> Result<()>,
        C: FnOnce(&str) -> Result<()>,
    {
        let _identity_operation = self.inner.identity_operation.lock().await;
        if !crate::identity::is_valid_app_user_id(new_app_user_id) {
            return Err(Error::new(
                ErrorCode::InvalidAppUserIdError,
                format!(
                    "appUserID must be non-blank and at most {} UTF-8 bytes.",
                    crate::identity::MAX_APP_USER_ID_BYTES
                ),
            ));
        }
        self.ensure_identity_transition_allowed(
            IdentityTransitionKind::LogIn,
            new_app_user_id,
            tracks_durable_transition,
        )?;
        let current = self.app_user_id();
        if current != new_app_user_id {
            self.ensure_native_purchase_recovery_complete()?;
        }
        if current == new_app_user_id {
            commit(&current)?;
            self.clear_durable_identity_transition(tracks_durable_transition);
            return Ok((
                self.get_customer_info_for_user(&current, CacheFetchPolicy::CachedOrFetched)
                    .await?,
                false,
            ));
        }
        self.mark_durable_identity_transition(
            IdentityTransitionKind::LogIn,
            new_app_user_id,
            tracks_durable_transition,
        );
        prepare(&current, new_app_user_id)?;
        let (info, created) = self.inner.backend.log_in(&current, new_app_user_id).await?;
        commit(new_app_user_id)?;
        self.inner.identity.switch_user(new_app_user_id);
        self.clear_durable_identity_transition(tracks_durable_transition);
        self.clear_identity_scoped_state();
        self.store_and_notify_for_user(new_app_user_id, &info);
        Ok((info, created))
    }

    /// Resets to a new anonymous user. Errors if already anonymous, matching
    /// `LogOutWithAnonymousUserError` in the official SDKs.
    pub async fn log_out(&self) -> Result<CustomerInfo> {
        self.log_out_internal(None, false, |_, _| Ok(()), |_| Ok(()))
            .await
    }

    /// Commits the next anonymous identity durably before exposing it to
    /// purchases. A commit error leaves the existing identified user active.
    #[doc(hidden)]
    pub async fn log_out_with_identity_transaction<P, C>(
        &self,
        prepare: P,
        commit: C,
    ) -> Result<CustomerInfo>
    where
        P: FnOnce(&str, &str) -> Result<()>,
        C: FnOnce(&str) -> Result<()>,
    {
        self.log_out_internal(None, true, prepare, commit).await
    }

    /// Resumes a durably prepared logout using its already-persisted
    /// anonymous identity instead of generating a second one.
    #[doc(hidden)]
    pub async fn resume_log_out_with_identity_transaction<P, C>(
        &self,
        pending_anonymous_app_user_id: &str,
        prepare: P,
        commit: C,
    ) -> Result<CustomerInfo>
    where
        P: FnOnce(&str, &str) -> Result<()>,
        C: FnOnce(&str) -> Result<()>,
    {
        if !crate::identity::is_anonymous_id(pending_anonymous_app_user_id) {
            return Err(Error::new(
                ErrorCode::InvalidAppUserIdError,
                "A resumed logout requires a valid RevenueCat anonymous App User ID.",
            ));
        }
        self.log_out_internal(Some(pending_anonymous_app_user_id), true, prepare, commit)
            .await
    }

    async fn log_out_internal<P, C>(
        &self,
        pending_anonymous_app_user_id: Option<&str>,
        tracks_durable_transition: bool,
        prepare: P,
        commit: C,
    ) -> Result<CustomerInfo>
    where
        P: FnOnce(&str, &str) -> Result<()>,
        C: FnOnce(&str) -> Result<()>,
    {
        let _identity_operation = self.inner.identity_operation.lock().await;
        if self.is_anonymous() {
            return Err(Error::new(
                ErrorCode::LogOutWithAnonymousUserError,
                "Called logOut but the current user is anonymous.",
            ));
        }
        self.ensure_native_purchase_recovery_complete()?;
        let pending_in_memory = tracks_durable_transition
            .then(|| {
                self.inner
                    .pending_identity_transition
                    .lock()
                    .expect("pending identity transition lock poisoned")
                    .as_ref()
                    .filter(|pending| pending.kind == IdentityTransitionKind::LogOut)
                    .map(|pending| pending.target.clone())
            })
            .flatten();
        let new_anonymous = pending_anonymous_app_user_id
            .or(pending_in_memory.as_deref())
            .map(str::to_owned)
            .unwrap_or_else(crate::identity::generate_anonymous_id);
        self.ensure_identity_transition_allowed(
            IdentityTransitionKind::LogOut,
            &new_anonymous,
            tracks_durable_transition,
        )?;
        let current = self.app_user_id();
        self.mark_durable_identity_transition(
            IdentityTransitionKind::LogOut,
            &new_anonymous,
            tracks_durable_transition,
        );
        prepare(&current, &new_anonymous)?;
        let info = self.inner.backend.get_customer_info(&new_anonymous).await?;
        commit(&new_anonymous)?;
        self.inner.identity.switch_user(&new_anonymous);
        self.clear_durable_identity_transition(tracks_durable_transition);
        self.clear_identity_scoped_state();
        self.store_and_notify_for_user(&new_anonymous, &info);
        Ok(info)
    }

    fn ensure_identity_transition_allowed(
        &self,
        kind: IdentityTransitionKind,
        target: &str,
        tracked: bool,
    ) -> Result<()> {
        let pending = self
            .inner
            .pending_identity_transition
            .lock()
            .expect("pending identity transition lock poisoned");
        match pending.as_ref() {
            None => Ok(()),
            Some(current) if tracked && current.kind == kind && current.target == target => Ok(()),
            Some(_) => Err(identity_operation_in_progress_error()),
        }
    }

    fn mark_durable_identity_transition(
        &self,
        kind: IdentityTransitionKind,
        target: &str,
        tracked: bool,
    ) {
        if tracked {
            *self
                .inner
                .pending_identity_transition
                .lock()
                .expect("pending identity transition lock poisoned") =
                Some(PendingIdentityTransition {
                    kind,
                    target: target.to_owned(),
                });
            self.inner
                .identity_transition_pending
                .store(true, Ordering::Release);
        }
    }

    fn clear_durable_identity_transition(&self, tracked: bool) {
        if tracked {
            *self
                .inner
                .pending_identity_transition
                .lock()
                .expect("pending identity transition lock poisoned") = None;
            self.inner
                .identity_transition_pending
                .store(false, Ordering::Release);
        }
    }

    fn ensure_native_purchase_recovery_complete(&self) -> Result<()> {
        if self
            .inner
            .native_purchase_recovery_required
            .load(Ordering::Acquire)
        {
            return Err(native_purchase_recovery_error());
        }
        Ok(())
    }

    fn clear_identity_scoped_state(&self) {
        self.inner.cache.clear();
        self.inner
            .store_purchase_options
            .lock()
            .expect("store purchase options lock poisoned")
            .clear_identity_scoped_options();
        self.inner
            .launched_store_plans
            .lock()
            .expect("launched store plans lock poisoned")
            .clear();
    }
}

fn identity_operation_in_progress_error() -> Error {
    Error::new(
        ErrorCode::OperationAlreadyInProgressError,
        "A durable App User ID transition is incomplete. Retry the exact same logIn/logOut operation before starting another identity change.",
    )
}
