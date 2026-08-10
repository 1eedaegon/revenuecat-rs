//! HTTP client speaking the RevenueCat wire protocol: Bearer auth, `X-*`
//! device headers, custom ETag caching, `{"code", "message"}` error bodies,
//! and Trusted Entitlements signature verification. Mirrors `HTTPClient`
//! from purchases-android/ios.

use std::time::Duration;

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde_json::Value;

use std::sync::Arc;
use std::time::Instant;

use crate::configuration::{Configuration, EntitlementVerificationMode};
use crate::diagnostics::DiagnosticsTracker;
use crate::error::{Error, ErrorCode, Result};
use crate::http::etag::ETagManager;
use crate::models::VerificationResult;
use crate::security::{
    generate_nonce, post_params_hash, SignatureVerifier, VerifyParams, ROOT_PUBLIC_KEY_B64,
};

pub const SIGNATURE_HEADER: &str = "X-Signature";
pub const REQUEST_TIME_HEADER: &str = "X-RevenueCat-Request-Time";
pub const NONCE_HEADER: &str = "X-Nonce";
pub const POST_PARAMS_HASH_HEADER: &str = "X-Post-Params-Hash";

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

/// Per-request protocol options, mirroring purchases-android's per-endpoint
/// `isRetryable` / `supportsSignatureVerification` / `needsNonceToPerformSigning`
/// flags plus `postFieldsToSign`.
#[derive(Debug, Default)]
pub struct RequestOptions {
    pub retryable: bool,
    /// Endpoint supports Trusted Entitlements verification.
    pub verify: bool,
    /// Endpoint sends an `X-Nonce` when verification is enabled.
    pub nonce: bool,
    /// `(field_name, value)` pairs hashed into `X-Post-Params-Hash`.
    pub signed_fields: Vec<(&'static str, String)>,
    /// Android-style endpoint name for the `http_request_performed`
    /// diagnostics event; `None` skips tracking.
    pub endpoint_name: Option<&'static str>,
}

impl RequestOptions {
    pub fn verified() -> Self {
        Self {
            verify: true,
            ..Self::default()
        }
    }

    pub fn verified_with_nonce() -> Self {
        Self {
            verify: true,
            nonce: true,
            ..Self::default()
        }
    }
}

#[derive(Debug)]
pub struct HttpClient {
    client: reqwest::Client,
    base_url: String,
    default_headers: Vec<(&'static str, String)>,
    etags: ETagManager,
    verifier: Option<SignatureVerifier>,
    mode: EntitlementVerificationMode,
    diagnostics: Arc<DiagnosticsTracker>,
}

#[derive(Debug)]
pub struct HttpResponse<T> {
    pub value: T,
    pub status: u16,
    /// Trusted Entitlements verification outcome for this response.
    pub verification: VerificationResult,
}

impl HttpClient {
    pub fn new(config: &Configuration, diagnostics: Arc<DiagnosticsTracker>) -> Result<Self> {
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

        let verifier = if config.verification_mode.is_enabled() {
            let root = config
                .verification_root_key
                .as_deref()
                .unwrap_or(ROOT_PUBLIC_KEY_B64);
            Some(
                SignatureVerifier::new(root, &config.api_key)
                    .map_err(|e| Error::with_underlying(ErrorCode::ConfigurationError, e))?,
            )
        } else {
            None
        };

        Ok(Self {
            client,
            base_url: config.base_url.clone(),
            default_headers,
            etags: ETagManager::new(),
            verifier,
            mode: config.verification_mode,
            diagnostics,
        })
    }

    /// POST to an absolute URL on a different host (diagnostics), without
    /// ETag, verification, or self-tracking.
    pub async fn post_absolute(&self, url: &str, body: Value) -> Result<Value> {
        let mut request = self.client.post(url);
        for (name, value) in &self.default_headers {
            request = request.header(*name, value);
        }
        let response = request.json(&body).send().await?;
        let status = response.status().as_u16();
        let text = response.text().await?;
        Ok(Self::finish::<Value>(status, &text, VerificationResult::NotRequested)?.value)
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Ok(self.get_with(path, RequestOptions::default()).await?.value)
    }

    pub async fn get_with<T: DeserializeOwned>(
        &self,
        path: &str,
        options: RequestOptions,
    ) -> Result<HttpResponse<T>> {
        self.request(Method::GET, path, None, options).await
    }

    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: Value) -> Result<T> {
        Ok(self
            .post_with(path, body, RequestOptions::default())
            .await?
            .value)
    }

    pub async fn post_with<T: DeserializeOwned>(
        &self,
        path: &str,
        body: Value,
        options: RequestOptions,
    ) -> Result<HttpResponse<T>> {
        self.request(Method::POST, path, Some(body), options).await
    }

    fn verifying(&self, options: &RequestOptions) -> bool {
        options.verify && self.verifier.is_some()
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        options: RequestOptions,
    ) -> Result<HttpResponse<T>> {
        let url = format!("{}{}", self.base_url, path);
        let use_etag = method == Method::GET;
        let post_params_hash_value =
            if self.verifying(&options) && !options.signed_fields.is_empty() {
                let fields: Vec<(&str, &str)> = options
                    .signed_fields
                    .iter()
                    .map(|(name, value)| (*name, value.as_str()))
                    .collect();
                Some(post_params_hash(&fields))
            } else {
                None
            };

        let mut attempt = 0;
        loop {
            let outcome = self
                .perform(
                    method.clone(),
                    path,
                    &url,
                    body.as_ref(),
                    use_etag,
                    /* force_refresh */ false,
                    &options,
                    post_params_hash_value.as_deref(),
                )
                .await?;
            match outcome {
                Outcome::Resolved {
                    status,
                    body,
                    verification,
                } => {
                    return Self::finish::<T>(status, &body, verification);
                }
                Outcome::NotModifiedWithoutCache => {
                    // Mirror ETagManager: one retry with an empty ETag header.
                    let retried = self
                        .perform(
                            method.clone(),
                            path,
                            &url,
                            body.as_ref(),
                            use_etag,
                            true,
                            &options,
                            post_params_hash_value.as_deref(),
                        )
                        .await?;
                    return match retried {
                        Outcome::Resolved {
                            status,
                            body,
                            verification,
                        } => Self::finish::<T>(status, &body, verification),
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
                    if !options.retryable || attempt + 1 >= RETRY_BACKOFF.len() {
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

    #[allow(clippy::too_many_arguments)]
    async fn perform(
        &self,
        method: Method,
        path: &str,
        url: &str,
        body: Option<&Value>,
        use_etag: bool,
        force_refresh: bool,
        options: &RequestOptions,
        post_params_hash_value: Option<&str>,
    ) -> Result<Outcome> {
        let mut request = self.client.request(method, url);
        for (name, value) in &self.default_headers {
            request = request.header(*name, value);
        }

        let verifying = self.verifying(options);
        let nonce = if verifying && options.nonce {
            Some(generate_nonce())
        } else {
            None
        };
        if let Some(nonce) = &nonce {
            request = request.header(NONCE_HEADER, nonce);
        }
        if let Some(hash) = post_params_hash_value {
            request = request.header(POST_PARAMS_HASH_HEADER, hash);
        }
        if use_etag {
            // A cached body may only be revalidated via ETag when it was
            // verified (or verification is off), per `shouldUseETag`.
            for (name, value) in self.etags.request_headers(url, force_refresh, verifying) {
                request = request.header(name, value);
            }
        }
        if let Some(body) = body {
            request = request.json(body);
        }

        let started = Instant::now();
        let response = request.send().await.inspect_err(|_| {
            self.track_request(options, started, 0, None, VerificationResult::NotRequested);
        })?;
        let status = response.status().as_u16();
        let etag = response
            .headers()
            .get(super::etag::ETAG_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let signature = response
            .headers()
            .get(SIGNATURE_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let request_time = response
            .headers()
            .get(REQUEST_TIME_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        if status == 304 {
            let verification = self.verify_response(
                options,
                path,
                nonce.as_deref(),
                post_params_hash_value,
                signature.as_deref(),
                request_time.as_deref(),
                etag.as_deref(),
                b"",
            )?;
            self.track_request(options, started, status, None, verification);
            return Ok(match self.etags.get(url) {
                // The cached body is replayed with the FRESH 304 verification
                // result, mirroring `ETagManager.getHTTPResultFromCacheOrBackend`.
                Some(cached) => Outcome::Resolved {
                    status: cached.status,
                    body: cached.body,
                    verification,
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

        let body = response.text().await?;
        let backend_error_code = if status >= 400 {
            serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.get("code").and_then(Value::as_i64))
        } else {
            None
        };
        let verification = if (200..300).contains(&status) {
            self.verify_response(
                options,
                path,
                nonce.as_deref(),
                post_params_hash_value,
                signature.as_deref(),
                request_time.as_deref(),
                etag.as_deref(),
                body.as_bytes(),
            )?
        } else {
            VerificationResult::NotRequested
        };

        self.track_request(options, started, status, backend_error_code, verification);

        // FAILED responses are never cached (`shouldStoreBackendResult`).
        if use_etag && (200..300).contains(&status) && verification != VerificationResult::Failed {
            if let Some(etag) = etag.filter(|e| !e.is_empty()) {
                self.etags.store(url, &etag, status, &body, verification);
            }
        }

        Ok(Outcome::Resolved {
            status,
            body,
            verification,
        })
    }

    fn track_request(
        &self,
        options: &RequestOptions,
        started: Instant,
        status: u16,
        backend_error_code: Option<i64>,
        verification: VerificationResult,
    ) {
        let Some(endpoint_name) = options.endpoint_name else {
            return;
        };
        if !self.diagnostics.is_enabled() {
            return;
        }
        let host = self
            .base_url
            .split("//")
            .nth(1)
            .unwrap_or(&self.base_url)
            .to_owned();
        let verification_name = match verification {
            VerificationResult::NotRequested => "NOT_REQUESTED",
            VerificationResult::Verified => "VERIFIED",
            VerificationResult::VerifiedOnDevice => "VERIFIED_ON_DEVICE",
            VerificationResult::Failed => "FAILED",
        };
        self.diagnostics.track_http_request(
            &host,
            endpoint_name,
            started.elapsed().as_millis() as i64,
            /* successful */ status != 0 && status < 400,
            status,
            backend_error_code,
            /* etag_hit */ status == 304,
            verification_name,
        );
    }

    /// Mirrors `SigningManager.verifyResponse`: decides
    /// NotRequested/Verified/Failed and, in Enforced mode, turns failures
    /// into `SignatureVerificationError`.
    #[allow(clippy::too_many_arguments)]
    fn verify_response(
        &self,
        options: &RequestOptions,
        path: &str,
        nonce_b64: Option<&str>,
        post_params_hash_value: Option<&str>,
        signature: Option<&str>,
        request_time: Option<&str>,
        etag: Option<&str>,
        body: &[u8],
    ) -> Result<VerificationResult> {
        let Some(verifier) = &self.verifier else {
            return Ok(VerificationResult::NotRequested);
        };
        if !options.verify {
            return Ok(VerificationResult::NotRequested);
        }

        let outcome = (|| -> std::result::Result<(), String> {
            let signature = signature.ok_or("missing X-Signature header")?;
            let request_time = request_time.ok_or("missing X-RevenueCat-Request-Time header")?;
            if body.is_empty() && etag.is_none() {
                return Err("response has neither a body nor an ETag".to_owned());
            }
            let url_path = path.split('?').next().unwrap_or(path);
            verifier.verify(
                signature,
                &VerifyParams {
                    nonce_b64,
                    url_path,
                    post_params_hash: post_params_hash_value,
                    request_time,
                    etag,
                    body,
                },
            )
        })();

        match outcome {
            Ok(()) => Ok(VerificationResult::Verified),
            Err(reason) => {
                log::warn!("Trusted Entitlements verification failed for {path}: {reason}");
                if self.mode == EntitlementVerificationMode::Enforced {
                    Err(Error::with_underlying(
                        ErrorCode::SignatureVerificationError,
                        reason,
                    ))
                } else {
                    Ok(VerificationResult::Failed)
                }
            }
        }
    }

    fn finish<T: DeserializeOwned>(
        status: u16,
        body: &str,
        verification: VerificationResult,
    ) -> Result<HttpResponse<T>> {
        if (200..300).contains(&status) {
            let value = if body.is_empty() {
                serde_json::from_value(Value::Object(Default::default()))?
            } else {
                serde_json::from_str(body)?
            };
            return Ok(HttpResponse {
                value,
                status,
                verification,
            });
        }
        Err(parse_backend_error(status, body))
    }
}

#[derive(Debug)]
enum Outcome {
    Resolved {
        status: u16,
        body: String,
        verification: VerificationResult,
    },
    NotModifiedWithoutCache,
    RateLimited {
        retry_after: Option<Duration>,
    },
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
    error.error_body = Some(parsed);
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
    fn keeps_error_body_for_typed_flows() {
        let body = r#"{"code": 7853, "message": "The link has expired.",
            "purchase_redemption_error_info": {"obfuscated_email": "g***@e.com"}}"#;
        let error = parse_backend_error(403, body);
        assert_eq!(
            error.error_body.as_ref().unwrap()["purchase_redemption_error_info"]
                ["obfuscated_email"],
            "g***@e.com"
        );
    }

    #[test]
    fn unparseable_5xx_maps_to_unknown_backend_error() {
        let error = parse_backend_error(503, "upstream connect error");
        assert_eq!(error.code, ErrorCode::UnknownBackendError);
    }
}
