//! In-process mock of the RevenueCat backend.
//!
//! Implements the subset of the wire protocol the `revenuecat` crate speaks —
//! subscribers, receipts, offerings, Web Billing products, identify, and
//! subscriber attributes — with the same envelope shapes, custom ETag
//! headers, and `{"code", "message"}` error bodies as the real API. Used by
//! the SDK's integration tests and embedded in the Tauri demo app.

mod server;
mod sign;
mod state;

use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub use sign::test_root_public_key_b64;
pub use state::{
    MockOffering, MockPackage, MockProduct, MockRedemption, MockVirtualCurrency, RecordedRequest,
    ServerState, Subscriber,
};

use std::collections::HashMap;

/// Builder for a mock RevenueCat backend with a product/offering catalog.
#[derive(Debug, Default)]
pub struct MockRevenueCat {
    products: Vec<MockProduct>,
    offerings: Vec<MockOffering>,
    current_offering_id: Option<String>,
    virtual_currencies: Vec<MockVirtualCurrency>,
    redemptions: HashMap<String, MockRedemption>,
    tamper_signatures: bool,
}

impl MockRevenueCat {
    pub fn builder() -> Self {
        Self::default()
    }

    /// A catalog preset matching the purchases-js test fixtures: one current
    /// offering with a monthly subscription (`pro` entitlement) and a
    /// consumable.
    pub fn with_default_catalog() -> Self {
        Self::builder()
            .product(MockProduct::subscription(
                "monthly",
                "Monthly Pro",
                3_000_000,
                "USD",
                "P1M",
                "pro",
            ))
            .product(MockProduct::subscription(
                "annual",
                "Annual Pro",
                30_000_000,
                "USD",
                "P1Y",
                "pro",
            ))
            .product(MockProduct::consumable(
                "coins_100",
                "100 Coins",
                1_000_000,
                "USD",
            ))
            .offering(MockOffering {
                identifier: "default".into(),
                description: "Standard offering".into(),
                packages: vec![
                    MockPackage {
                        identifier: "$rc_monthly".into(),
                        product_id: "monthly".into(),
                    },
                    MockPackage {
                        identifier: "$rc_annual".into(),
                        product_id: "annual".into(),
                    },
                    MockPackage {
                        identifier: "coins".into(),
                        product_id: "coins_100".into(),
                    },
                ],
            })
            .current_offering("default")
            .virtual_currency("GLD", "Gold", 100)
            .redemption_token("rdm_valid", "annual")
            .expired_redemption_token("rdm_expired", "g***@e*****e.com")
    }

    pub fn product(mut self, product: MockProduct) -> Self {
        self.products.push(product);
        self
    }

    pub fn offering(mut self, offering: MockOffering) -> Self {
        self.offerings.push(offering);
        self
    }

    pub fn current_offering(mut self, id: impl Into<String>) -> Self {
        self.current_offering_id = Some(id.into());
        self
    }

    pub fn virtual_currency(mut self, code: &str, name: &str, balance: i64) -> Self {
        self.virtual_currencies.push(MockVirtualCurrency {
            code: code.into(),
            name: name.into(),
            balance,
        });
        self
    }

    /// Registers a one-time redemption token granting `product_id`.
    pub fn redemption_token(mut self, token: &str, product_id: &str) -> Self {
        self.redemptions.insert(
            token.into(),
            MockRedemption::Valid {
                product_id: product_id.into(),
            },
        );
        self
    }

    /// Registers a token that always answers "expired" (backend 7853).
    pub fn expired_redemption_token(mut self, token: &str, obfuscated_email: &str) -> Self {
        self.redemptions.insert(
            token.into(),
            MockRedemption::Expired {
                obfuscated_email: obfuscated_email.into(),
            },
        );
        self
    }

    /// Corrupts every response signature — for testing SDK failure paths.
    pub fn tamper_signatures(mut self) -> Self {
        self.tamper_signatures = true;
        self
    }

    /// Binds to an ephemeral localhost port and starts serving.
    pub async fn spawn(self) -> std::io::Result<MockServerHandle> {
        let state = Arc::new(ServerState::new(
            self.products,
            self.offerings,
            self.current_offering_id,
            self.virtual_currencies,
            self.redemptions,
            self.tamper_signatures,
        ));
        let router = server::router(Arc::clone(&state));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let task = tokio::spawn(async move {
            // Serve until the handle aborts the task.
            let _ = axum::serve(listener, router).await;
        });
        Ok(MockServerHandle {
            url: format!("http://{addr}"),
            state,
            task,
        })
    }
}

/// Running mock server. Aborts the serve task on drop.
pub struct MockServerHandle {
    pub url: String,
    pub state: Arc<ServerState>,
    task: JoinHandle<()>,
}

impl MockServerHandle {
    /// Answers the next `count` receipt posts with HTTP 429 (Retry-After: 0)
    /// to exercise the SDK's retry backoff.
    pub fn rate_limit_receipts(&self, count: u32) {
        self.state
            .receipt_rate_limits
            .store(count, std::sync::atomic::Ordering::SeqCst);
    }

    /// Forces the next ETag-capable GET on `path` to answer 304 even without
    /// a matching request ETag — the "304 without local cache" edge.
    pub fn force_304_next(&self, path: &str) {
        if let Ok(mut set) = self.state.force_not_modified.lock() {
            set.insert(path.to_owned());
        }
    }

    /// Requests received so far (method, path, headers subset, JSON body) —
    /// for asserting exact wire behavior in tests, in the spirit of
    /// purchases-js's MSW request spies.
    pub fn received(&self) -> Vec<RecordedRequest> {
        self.state.received()
    }

    pub fn received_on(&self, path_prefix: &str) -> Vec<RecordedRequest> {
        self.received()
            .into_iter()
            .filter(|r| r.path.starts_with(path_prefix))
            .collect()
    }
}

impl Drop for MockServerHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}
