//! Diagnostics tracking, mirroring `DiagnosticsTracker`/`DiagnosticsSynchronizer`
//! in purchases-android: entries are queued in memory and POSTed in batches of
//! up to 200 to the dedicated diagnostics host as `{"entries": [...]}`.
//! Off by default; 5xx failures retry up to 3 times before clearing, any
//! other failure clears immediately (Android semantics).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};
use uuid::Uuid;

pub const DEFAULT_DIAGNOSTICS_URL: &str = "https://api-diagnostics.revenuecat.com";
/// `MAX_EVENTS_TO_SYNC_PER_REQUEST` in `DiagnosticsSynchronizer.kt`.
pub const MAX_EVENTS_PER_REQUEST: usize = 200;
/// `MAX_NUMBER_POST_RETRIES` in `DiagnosticsSynchronizer.kt`.
pub const MAX_SYNC_RETRIES: u32 = 3;
/// In-memory stand-in for Android's 500 KB file cap.
const MAX_STORED_ENTRIES: usize = 2000;

#[derive(Debug)]
pub(crate) struct DiagnosticsTracker {
    enabled: bool,
    app_session_id: Uuid,
    entries: Mutex<Vec<Value>>,
    consecutive_failures: AtomicU32,
    syncing: AtomicBool,
}

impl DiagnosticsTracker {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            app_session_id: Uuid::new_v4(),
            entries: Mutex::new(Vec::new()),
            consecutive_failures: AtomicU32::new(0),
            syncing: AtomicBool::new(false),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Serializes one entry with the exact `DiagnosticsEntry` field set:
    /// id, version=1, name, properties, app_session_id, timestamp.
    pub fn track(&self, name: &str, properties: Value) {
        if !self.enabled {
            return;
        }
        let entry = json!({
            "id": Uuid::new_v4().to_string(),
            "version": 1,
            "name": name,
            "properties": properties,
            "app_session_id": self.app_session_id.to_string(),
            // Android's Iso8601Utils.format: millisecond precision, literal Z.
            "timestamp": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        });
        let mut entries = self.entries.lock().expect("diagnostics lock poisoned");
        if entries.len() >= MAX_STORED_ENTRIES {
            entries.clear();
            drop(entries);
            self.track("max_events_stored_limit_reached", json!({}));
            return;
        }
        entries.push(entry);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn track_http_request(
        &self,
        host: &str,
        endpoint_name: &str,
        response_time_millis: i64,
        successful: bool,
        response_code: u16,
        backend_error_code: Option<i64>,
        etag_hit: bool,
        verification_result: &str,
    ) {
        let mut properties = json!({
            "host": host,
            "endpoint_name": endpoint_name,
            "response_time_millis": response_time_millis,
            "successful": successful,
            "response_code": response_code,
            "etag_hit": etag_hit,
            "verification_result": verification_result,
            "is_retry": false,
        });
        if let Some(code) = backend_error_code {
            properties["backend_error_code"] = json!(code);
        }
        self.track("http_request_performed", properties);
    }

    /// Number of entries waiting to be synced.
    pub fn pending(&self) -> usize {
        self.entries
            .lock()
            .expect("diagnostics lock poisoned")
            .len()
    }

    /// Takes up to one request's worth of entries; restore on 5xx via
    /// `restore_batch`. Returns None when another sync is in flight
    /// (single-flight, like `isSyncing`).
    pub fn take_batch(&self) -> Option<Vec<Value>> {
        if self.syncing.swap(true, Ordering::SeqCst) {
            return None;
        }
        let mut entries = self.entries.lock().expect("diagnostics lock poisoned");
        if entries.is_empty() {
            self.syncing.store(false, Ordering::SeqCst);
            return None;
        }
        let count = entries.len().min(MAX_EVENTS_PER_REQUEST);
        Some(entries.drain(..count).collect())
    }

    /// Success (2xx): batch stays consumed, failure counter resets.
    pub fn batch_succeeded(&self) {
        self.consecutive_failures.store(0, Ordering::SeqCst);
        self.syncing.store(false, Ordering::SeqCst);
    }

    /// Retryable failure (5xx / transport): requeue; after MAX_SYNC_RETRIES
    /// consecutive failures, drop everything (Android behavior).
    pub fn batch_failed_retryable(&self, batch: Vec<Value>) {
        let failures = self.consecutive_failures.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut entries = self.entries.lock().expect("diagnostics lock poisoned");
            if failures >= MAX_SYNC_RETRIES {
                entries.clear();
            } else {
                let mut restored = batch;
                restored.append(&mut entries);
                *entries = restored;
            }
        }
        self.syncing.store(false, Ordering::SeqCst);
        if failures >= MAX_SYNC_RETRIES {
            self.consecutive_failures.store(0, Ordering::SeqCst);
            self.track("max_diagnostics_sync_retries_reached", json!({}));
        }
    }

    /// Non-retryable failure (4xx): drop everything immediately.
    pub fn batch_failed_fatal(&self) {
        self.entries
            .lock()
            .expect("diagnostics lock poisoned")
            .clear();
        self.consecutive_failures.store(0, Ordering::SeqCst);
        self.syncing.store(false, Ordering::SeqCst);
        self.track("clearing_diagnostics_after_failed_sync", json!({}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_tracker_records_nothing() {
        let tracker = DiagnosticsTracker::new(false);
        tracker.track("http_request_performed", json!({}));
        assert_eq!(tracker.pending(), 0);
    }

    #[test]
    fn entries_carry_the_android_field_set() {
        // Arrange
        let tracker = DiagnosticsTracker::new(true);

        // Act
        tracker.track_http_request(
            "api.revenuecat.com",
            "get_customer",
            123,
            true,
            200,
            None,
            false,
            "VERIFIED",
        );

        // Assert
        let batch = tracker.take_batch().unwrap();
        let entry = &batch[0];
        assert_eq!(entry["version"], 1);
        assert_eq!(entry["name"], "http_request_performed");
        assert_eq!(entry["properties"]["endpoint_name"], "get_customer");
        assert_eq!(entry["properties"]["verification_result"], "VERIFIED");
        assert!(entry["timestamp"].as_str().unwrap().ends_with('Z'));
        assert!(entry["id"].as_str().is_some());
        assert!(entry["app_session_id"].as_str().is_some());
    }

    #[test]
    fn retryable_failures_requeue_then_clear_after_three() {
        // Arrange
        let tracker = DiagnosticsTracker::new(true);
        tracker.track("http_request_performed", json!({}));

        // Act / Assert: two 5xx failures keep the entries queued...
        for _ in 0..2 {
            let batch = tracker.take_batch().unwrap();
            tracker.batch_failed_retryable(batch);
            assert_eq!(tracker.pending(), 1);
        }
        // ...the third clears the queue and records the marker event.
        let batch = tracker.take_batch().unwrap();
        tracker.batch_failed_retryable(batch);
        let names: Vec<String> = tracker
            .take_batch()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap_or_default().to_owned())
            .collect();
        assert_eq!(names, vec!["max_diagnostics_sync_retries_reached"]);
    }

    #[test]
    fn fatal_failure_clears_and_records_marker() {
        let tracker = DiagnosticsTracker::new(true);
        tracker.track("http_request_performed", json!({}));
        let _batch = tracker.take_batch().unwrap();
        tracker.batch_failed_fatal();
        let names: Vec<String> = tracker
            .take_batch()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap_or_default().to_owned())
            .collect();
        assert_eq!(names, vec!["clearing_diagnostics_after_failed_sync"]);
    }

    #[test]
    fn take_batch_is_single_flight() {
        let tracker = DiagnosticsTracker::new(true);
        tracker.track("a", json!({}));
        let batch = tracker.take_batch().unwrap();
        assert!(tracker.take_batch().is_none(), "second sync must wait");
        tracker.batch_succeeded();
        let _ = batch;
    }
}
