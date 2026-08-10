//! App user id management, mirroring `IdentityManager` in the official SDKs.
//!
//! Anonymous ids follow the exact cross-platform format
//! `$RCAnonymousID:<32 lowercase hex>` validated by both purchases-android
//! (`IdentityManager.kt`) and purchases-ios (`IdentityManager.swift`).

use std::sync::RwLock;

use uuid::Uuid;

pub const ANONYMOUS_ID_PREFIX: &str = "$RCAnonymousID:";

pub fn generate_anonymous_id() -> String {
    format!("{ANONYMOUS_ID_PREFIX}{}", Uuid::new_v4().simple())
}

pub fn is_anonymous_id(app_user_id: &str) -> bool {
    match app_user_id.strip_prefix(ANONYMOUS_ID_PREFIX) {
        Some(rest) => {
            rest.len() == 32
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }
        None => false,
    }
}

#[derive(Debug)]
pub(crate) struct IdentityManager {
    current: RwLock<String>,
}

impl IdentityManager {
    /// `None` or a blank id configures an anonymous user, matching the
    /// official SDKs' `appUserID: null` behavior.
    pub fn new(app_user_id: Option<String>) -> Self {
        let id = match app_user_id {
            Some(id) if !id.trim().is_empty() => id,
            _ => generate_anonymous_id(),
        };
        Self {
            current: RwLock::new(id),
        }
    }

    pub fn current_app_user_id(&self) -> String {
        self.current.read().expect("identity lock poisoned").clone()
    }

    pub fn is_anonymous(&self) -> bool {
        is_anonymous_id(&self.current_app_user_id())
    }

    pub fn switch_user(&self, new_app_user_id: &str) {
        *self.current.write().expect("identity lock poisoned") = new_app_user_id.to_owned();
    }

    /// Resets to a fresh anonymous user and returns the new id.
    pub fn reset_to_anonymous(&self) -> String {
        let id = generate_anonymous_id();
        self.switch_user(&id);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_anonymous_ids() {
        // Arrange / Act
        let id = generate_anonymous_id();

        // Assert
        assert!(is_anonymous_id(&id), "generated id should validate: {id}");
        assert_eq!(id.len(), ANONYMOUS_ID_PREFIX.len() + 32);
    }

    #[test]
    fn rejects_non_anonymous_ids() {
        assert!(!is_anonymous_id("user-123"));
        assert!(!is_anonymous_id("$RCAnonymousID:"));
        assert!(!is_anonymous_id("$RCAnonymousID:XYZ"));
        // Uppercase hex is rejected by the SDK regex `[a-f0-9]{32}`.
        assert!(!is_anonymous_id(&format!(
            "{ANONYMOUS_ID_PREFIX}{}",
            "A".repeat(32)
        )));
    }

    #[test]
    fn blank_configured_id_falls_back_to_anonymous() {
        let manager = IdentityManager::new(Some("   ".into()));
        assert!(manager.is_anonymous());
    }

    #[test]
    fn switch_and_reset_user() {
        // Arrange
        let manager = IdentityManager::new(Some("gon".into()));
        assert!(!manager.is_anonymous());

        // Act
        manager.switch_user("dae");
        let after_switch = manager.current_app_user_id();
        let anon = manager.reset_to_anonymous();

        // Assert
        assert_eq!(after_switch, "dae");
        assert_eq!(manager.current_app_user_id(), anon);
        assert!(manager.is_anonymous());
    }
}
