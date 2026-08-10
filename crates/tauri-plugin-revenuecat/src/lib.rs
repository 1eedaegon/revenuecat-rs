//! # tauri-plugin-revenuecat
//!
//! Bridges the platform store (StoreKit 2 on iOS, Play Billing on Android)
//! into the `revenuecat` crate's [`StoreBilling`] seam through a Tauri 2
//! mobile plugin. Desktop targets have no store — configure the SDK with a
//! `test_` Test Store key there, or sell via web billing.
//!
//! ```ignore
//! tauri::Builder::default()
//!     .plugin(tauri_plugin_revenuecat::init())
//!     .setup(|app| {
//!         let billing = tauri_plugin_revenuecat::store_billing(app.handle())?;
//!         let purchases = revenuecat::Purchases::configure(
//!             revenuecat::Configuration::builder("goog_or_appl_KEY")
//!                 .store_billing(billing)
//!                 .build()?,
//!         )?;
//!         // manage(purchases) ...
//!         Ok(())
//!     })
//! ```
//!
//! Status: the Rust side and native shims are code-complete; end-to-end
//! purchases still need store accounts + devices to verify (see README).

#[cfg(mobile)]
mod mobile;
mod models;

use std::sync::Arc;

use revenuecat::StoreBilling;
use tauri::plugin::{Builder, TauriPlugin};
#[cfg(mobile)]
use tauri::Manager;
use tauri::{AppHandle, Runtime};

pub use models::{PluginProduct, PluginTransaction};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_revenuecat);

/// State managed by the plugin on mobile targets.
#[cfg(mobile)]
struct RevenuecatExt<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Initializes the plugin; registers the native shim on mobile targets and
/// is a no-op on desktop.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("revenuecat")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                let handle =
                    _api.register_android_plugin("app.tauri.revenuecat", "RevenueCatPlugin")?;
                _app.manage(RevenuecatExt(handle));
            }
            #[cfg(target_os = "ios")]
            {
                let handle = _api.register_ios_plugin(init_plugin_revenuecat)?;
                _app.manage(RevenuecatExt(handle));
            }
            Ok(())
        })
        .build()
}

/// Returns a [`StoreBilling`] backed by the platform store, for
/// `ConfigurationBuilder::store_billing`. Errors on desktop targets, where
/// no store exists.
pub fn store_billing<R: Runtime>(app: &AppHandle<R>) -> revenuecat::Result<Arc<dyn StoreBilling>> {
    #[cfg(mobile)]
    {
        let ext = app.try_state::<RevenuecatExt<R>>().ok_or_else(|| {
            revenuecat::Error::new(
                revenuecat::ErrorCode::ConfigurationError,
                "tauri-plugin-revenuecat is not initialized; add .plugin(tauri_plugin_revenuecat::init()).",
            )
        })?;
        Ok(Arc::new(mobile::MobileStoreBilling::new(ext.0.clone())))
    }
    #[cfg(not(mobile))]
    {
        let _ = app;
        Err(revenuecat::Error::new(
            revenuecat::ErrorCode::UnsupportedError,
            "No platform store on desktop. Use a `test_` Test Store key, or sell via \
             RevenueCat Web Billing and attach purchases with redeem_web_purchase.",
        ))
    }
}

#[cfg(all(test, not(mobile)))]
mod tests {
    #[test]
    fn desktop_has_no_store_billing() {
        let app = tauri::test::mock_app();
        let error = match super::store_billing(app.handle()) {
            Err(error) => error,
            Ok(_) => panic!("desktop must not provide a store"),
        };
        assert_eq!(error.code, revenuecat::ErrorCode::UnsupportedError);
    }
}
