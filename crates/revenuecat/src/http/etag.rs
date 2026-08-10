//! RevenueCat's custom ETag protocol, mirroring `ETagManager` in both mobile
//! SDKs. It does NOT use standard `If-None-Match`/`ETag` headers:
//!
//! - request:  `X-RevenueCat-ETag: <etag or empty>`, `X-RC-Last-Refresh-Time: <ms>`
//! - response: `X-RevenueCat-ETag: <etag>`; HTTP 304 means "replay your copy"
//! - a 304 without a local copy is retried once with an empty ETag header.

use std::collections::HashMap;
use std::sync::Mutex;

pub const ETAG_HEADER: &str = "X-RevenueCat-ETag";
pub const LAST_REFRESH_TIME_HEADER: &str = "X-RC-Last-Refresh-Time";

#[derive(Debug, Clone)]
pub struct CachedHttpResponse {
    pub etag: String,
    pub status: u16,
    pub body: String,
    pub stored_at_ms: i64,
}

#[derive(Debug, Default)]
pub struct ETagManager {
    entries: Mutex<HashMap<String, CachedHttpResponse>>,
}

impl ETagManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Header values for an outgoing request. An empty ETag (initial request
    /// or forced refresh) tells the backend to send a full response.
    pub fn request_headers(&self, url: &str, force_refresh: bool) -> Vec<(&'static str, String)> {
        let cached = if force_refresh { None } else { self.get(url) };
        match cached {
            Some(entry) => vec![
                (ETAG_HEADER, entry.etag),
                (LAST_REFRESH_TIME_HEADER, entry.stored_at_ms.to_string()),
            ],
            None => vec![(ETAG_HEADER, String::new())],
        }
    }

    pub fn get(&self, url: &str) -> Option<CachedHttpResponse> {
        self.entries
            .lock()
            .expect("etag lock poisoned")
            .get(url)
            .cloned()
    }

    pub fn store(&self, url: &str, etag: &str, status: u16, body: &str) {
        let entry = CachedHttpResponse {
            etag: etag.to_owned(),
            status,
            body: body.to_owned(),
            stored_at_ms: chrono_now_ms(),
        };
        self.entries
            .lock()
            .expect("etag lock poisoned")
            .insert(url.to_owned(), entry);
    }
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_request_sends_empty_etag() {
        // Arrange
        let manager = ETagManager::new();

        // Act
        let headers = manager.request_headers("https://x/v1/subscribers/a", false);

        // Assert
        assert_eq!(headers, vec![(ETAG_HEADER, String::new())]);
    }

    #[test]
    fn cached_etag_is_sent_with_last_refresh_time() {
        // Arrange
        let manager = ETagManager::new();
        manager.store("u", "abc123", 200, "{}");

        // Act
        let headers = manager.request_headers("u", false);

        // Assert
        assert_eq!(headers[0], (ETAG_HEADER, "abc123".to_owned()));
        assert_eq!(headers[1].0, LAST_REFRESH_TIME_HEADER);
        assert!(headers[1].1.parse::<i64>().unwrap() > 0);
    }

    #[test]
    fn force_refresh_ignores_cache() {
        let manager = ETagManager::new();
        manager.store("u", "abc123", 200, "{}");
        let headers = manager.request_headers("u", true);
        assert_eq!(headers, vec![(ETAG_HEADER, String::new())]);
    }

    #[test]
    fn stores_and_replays_body() {
        let manager = ETagManager::new();
        manager.store("u", "tag", 200, r#"{"ok":true}"#);
        let cached = manager.get("u").unwrap();
        assert_eq!(cached.body, r#"{"ok":true}"#);
        assert_eq!(cached.status, 200);
    }
}
