//! In-process mock of the RevenueCat backend.
//!
//! Implements the subset of the wire protocol the `revenuecat` crate speaks —
//! subscribers, receipts, offerings, Web Billing products, identify, and
//! subscriber attributes — with the same envelope shapes, custom ETag
//! headers, and `{"code", "message"}` error bodies as the real API. Used by
//! the SDK's integration tests and embedded in the Tauri demo app.

mod server;
mod state;

use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub use state::{
    MockOffering, MockPackage, MockProduct, MockVirtualCurrency, RecordedRequest, ServerState,
    Subscriber,
};

/// Builder for a mock RevenueCat backend with a product/offering catalog.
#[derive(Debug, Default)]
pub struct MockRevenueCat {
    products: Vec<MockProduct>,
    offerings: Vec<MockOffering>,
    current_offering_id: Option<String>,
    virtual_currencies: Vec<MockVirtualCurrency>,
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

    /// Binds to an ephemeral localhost port and starts serving.
    pub async fn spawn(self) -> std::io::Result<MockServerHandle> {
        let state = Arc::new(ServerState::new(
            self.products,
            self.offerings,
            self.current_offering_id,
            self.virtual_currencies,
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
