//! `StoreTransaction`: the store-agnostic result of a purchase, whose
//! `purchase_token` becomes the `fetch_token` posted to `/v1/receipts`.

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::customer_info::Store;
use super::store_product::Price;

#[derive(Debug, Clone, Serialize)]
pub struct StoreTransaction {
    /// Store token proving the purchase. Simulated-store tokens look like
    /// `test_<epoch_ms>_<uuid>`, matching purchases-js / purchases-android.
    pub purchase_token: String,
    pub product_ids: Vec<String>,
    pub purchase_date: DateTime<Utc>,
    pub transaction_id: Option<String>,
    pub store: Store,
    /// Price paid, forwarded to the backend for revenue tracking.
    pub price: Option<Price>,
}
