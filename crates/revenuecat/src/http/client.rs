//! HTTP client speaking the RevenueCat wire protocol: Bearer auth, `X-*`
//! device headers, custom ETag caching, and `{"code", "message"}` error
//! bodies. Mirrors `HTTPClient` from purchases-android/ios.

use std::time::Duration;

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::configuration::Configuration;
use crate::error::{Error, ErrorCode, Result};
use crate::http::etag::ETagManager;

/// Backoff schedule for retryable requests (iOS `HTTPClient` retries
/// `POST /v1/receipts` on 429 with 0 / 0.75s / 3s delays, max 3 attempts).
const RETRY_BACKOFF: [Duration; 3] = [
    Duration::from_millis(0),
    Duration::from_millis(750),
    Duration::from_secs(3),
];

/// RFC 3986 unreserved characters stay literal, matching
/// `encodeURIComponent` (purchases-js) and `Uri.encode` (purchases-android).
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

pub fn encode_path_segment(segment: &str) -> String {
    utf8_percent_encode(segment, PATH_SEGMENT).to_string()
}

#[derive(Debug)]
pub struct HttpClient {
    client: reqwest::Client,
    base_url: String,
    default_headers: Vec<(&'static str, String)>,
    etags: ETagManager,
}

#[derive(Debug)]
pub struct HttpResponse<T> {
    pub value: T,
    pub status: u16,
}

impl HttpClient {
    pub fn new(config: &Configuration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(config.http_timeout)
            .build()
            .map_err(|e| Error::with_underlying(ErrorCode::ConfigurationError, e.to_string()))?;

        let mut default_headers = vec![
            ("Authorization", format!("Bearer {}", config.api_key)),
            ("Content-Type", "application/json".to_owned()),
            ("X-Platform", config.platform.as_str().to_owned()),
            ("X-Version", env!("CARGO_PKG_VERSION").to_owned()),
            ("X-Observer-Mode-Enabled", "false".to_owned()),
        ];
        if let Some(flavor) = &config.platform_flavor {
            default_headers.push(("X-Platform-Flavor", flavor.clone()));
        }
        if let Some(version) = &config.platform_flavor_version {
            default_headers.push(("X-Platform-Flavor-Version", version.clone()));
        }

        Ok(Self {
            client,
            base_url: config.base_url.clone(),
            default_headers,
            etags: ETagManager::new(),
        })
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Ok(self.request(Method::GET, path, None, false).await?.value)
    }

    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: Value) -> Result<T> {
        Ok(self
            .request(Method::POST, path, Some(body), false)
            .await?
            .value)
    }

    /// POST that also surfaces the HTTP status (login needs 201 => "created")
    /// and retries on 429 like the official SDKs' receipt posting.
    pub async fn post_with_status<T: DeserializeOwned>(
        &self,
        path: &str,
        body: Value,
        retryable: bool,
    ) -> Result<HttpResponse<T>> {
        self.request(Method::POST, path, Some(body), retryable)
            .await
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        retryable: bool,
    ) -> Result<HttpResponse<T>> {
        let url = format!("{}{}", self.base_url, path);
        let use_etag = method == Method::GET;

        let mut attempt = 0;
        loop {
            let outcome = self
                .perform(
                    method.clone(),
                    &url,
                    body.as_ref(),
                    use_etag,
                    /* force_refresh */ false,
                )
                .await?;
            match outcome {
                Outcome::Resolved { status, body } => {
                    return Self::finish::<T>(status, &body);
                }
                Outcome::NotModifiedWithoutCache => {
                    // Mirror ETagManager: one retry with an empty ETag header.
                    let retried = self
                        .perform(method.clone(), &url, body.as_ref(), use_etag, true)
                        .await?;
                    return match retried {
                        Outcome::Resolved { status, body } => Self::finish::<T>(status, &body),
                        Outcome::NotModifiedWithoutCache => Err(Error::new(
                            ErrorCode::UnexpectedBackendResponseError,
                            "Received 304 without a cached response after ETag retry.",
                        )),
                        Outcome::RateLimited { .. } => Err(Error::new(
                            ErrorCode::NetworkError,
                            "Rate limited while retrying an ETag miss.",
                        )),
                    };
                }
                Outcome::RateLimited { retry_after } => {
                    if !retryable || attempt + 1 >= RETRY_BACKOFF.len() {
                        return Err(Error::new(
                            ErrorCode::NetworkError,
                            "The server is rate limiting requests (HTTP 429).",
                        ));
                    }
                    let delay = retry_after.unwrap_or(RETRY_BACKOFF[attempt + 1]);
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }

    async fn perform(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
        use_etag: bool,
        force_refresh: bool,
    ) -> Result<Outcome> {
        let mut request = self.client.request(method, url);
        for (name, value) in &self.default_headers {
            request = request.header(*name, value);
        }
        if use_etag {
            for (name, value) in self.etags.request_headers(url, force_refresh) {
                request = request.header(name, value);
            }
        }
        if let Some(body) = body {
            request = request.json(body);
        }

        let response = request.send().await?;
        let status = response.status().as_u16();

        if status == 304 {
            return Ok(match self.etags.get(url) {
                Some(cached) => Outcome::Resolved {
                    status: cached.status,
                    body: cached.body,
                },
                None => Outcome::NotModifiedWithoutCache,
            });
        }
        if status == 429 {
            let retry_after = response
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            return Ok(Outcome::RateLimited { retry_after });
        }

        let etag = response
            .headers()
            .get(super::etag::ETAG_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = response.text().await?;

        if use_etag && (200..300).contains(&status) {
            if let Some(etag) = etag.filter(|e| !e.is_empty()) {
                self.etags.store(url, &etag, status, &body);
            }
        }

        Ok(Outcome::Resolved { status, body })
    }

    fn finish<T: DeserializeOwned>(status: u16, body: &str) -> Result<HttpResponse<T>> {
        if (200..300).contains(&status) {
            let value = if body.is_empty() {
                serde_json::from_value(Value::Object(Default::default()))?
            } else {
                serde_json::from_str(body)?
            };
            return Ok(HttpResponse { value, status });
        }
        Err(parse_backend_error(status, body))
    }
}

#[derive(Debug)]
enum Outcome {
    Resolved { status: u16, body: String },
    NotModifiedWithoutCache,
    RateLimited { retry_after: Option<Duration> },
}

/// Backend errors look like `{"code": 7259, "message": "..."}` — `code` may
/// arrive as an int or a string (iOS accepts both). Subscriber-attribute
/// failures add `attribute_errors: [{key_name, message}]`, either at the top
/// level (attributes endpoint) or nested under `attributes_error_response`
/// (receipts endpoint).
fn parse_backend_error(status: u16, body: &str) -> Error {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let Some(parsed) = parsed else {
        return if status >= 500 {
            Error::new(
                ErrorCode::UnknownBackendError,
                format!("Internal server error (HTTP {status})."),
            )
        } else {
            Error::new(
                ErrorCode::UnexpectedBackendResponseError,
                format!("Unparseable backend error (HTTP {status}): {body}"),
            )
        };
    };

    let code = match parsed.get("code") {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.parse::<i64>().ok(),
        _ => None,
    };
    let message = parsed
        .get("message")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .map(str::to_owned);

    let mut error = match code.filter(|c| *c > 0) {
        Some(code) => Error::from_backend(
            code,
            message.unwrap_or_else(|| format!("Backend error (HTTP {status})")),
        ),
        None if status >= 500 => Error::new(
            ErrorCode::UnknownBackendError,
            format!("Internal server error (HTTP {status})."),
        ),
        None => Error::new(
            ErrorCode::UnknownBackendError,
            format!(
                "Backend Code: N/A - {}",
                message.unwrap_or_else(|| body.to_owned())
            ),
        ),
    };

    if let Some(attribute_errors) = extract_attribute_errors(&parsed) {
        error.underlying = Some(attribute_errors);
    }
    error
}

fn extract_attribute_errors(body: &Value) -> Option<String> {
    let container = body.get("attributes_error_response").unwrap_or(body);
    let errors = container.get("attribute_errors")?.as_array()?;
    let joined = errors
        .iter()
        .filter_map(|e| {
            Some(format!(
                "{}: {}",
                e.get("key_name")?.as_str()?,
                e.get("message")?.as_str()?
            ))
        })
        .collect::<Vec<_>>()
        .join("; ");
    (!joined.is_empty()).then_some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_anonymous_ids_for_url_paths() {
        let encoded = encode_path_segment("$RCAnonymousID:abc123");
        assert_eq!(encoded, "%24RCAnonymousID%3Aabc123");
    }

    #[test]
    fn parses_backend_error_with_code() {
        let error = parse_backend_error(400, r#"{"code": 7226, "message": "Invalid receipt."}"#);
        assert_eq!(error.backend_code, Some(7226));
        assert_eq!(error.message, "Invalid receipt.");
    }

    #[test]
    fn parses_attribute_errors() {
        let body = r#"{"code": 7263, "message": "Some attributes could not be saved.",
            "attribute_errors": [{"key_name": "$email", "message": "Email is not valid."}]}"#;
        let error = parse_backend_error(400, body);
        assert_eq!(
            error.underlying.as_deref(),
            Some("$email: Email is not valid.")
        );
    }

    #[test]
    fn unparseable_5xx_maps_to_unknown_backend_error() {
        let error = parse_backend_error(503, "upstream connect error");
        assert_eq!(error.code, ErrorCode::UnknownBackendError);
    }
}
