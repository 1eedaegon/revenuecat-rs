//! Axum router and handlers emulating the RevenueCat API subset.

use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::{Path, RawQuery, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::sign::expected_post_params_hash;
use crate::state::{
    expiry_for, MockRedemption, NonSubscriptionEntry, RecordedRequest, ServerState,
    SubscriptionEntry,
};

const ETAG_HEADER: &str = "X-RevenueCat-ETag";
const RECORDED_HEADERS: &[&str] = &[
    "authorization",
    "content-type",
    "x-platform",
    "x-platform-flavor",
    "x-version",
    "x-observer-mode-enabled",
    "x-revenuecat-etag",
    "x-nonce",
    "x-post-params-hash",
];

pub fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/v1/subscribers/{id}", get(get_subscriber))
        .route("/v1/subscribers/{id}/offerings", get(get_offerings))
        .route("/v1/subscribers/{id}/attributes", post(post_attributes))
        .route(
            "/v1/subscribers/{id}/virtual_currencies",
            get(get_virtual_currencies),
        )
        .route("/v1/subscribers/identify", post(post_identify))
        .route("/v1/subscribers/redeem_purchase", post(post_redeem))
        .route("/v1/receipts", post(post_receipts))
        .route("/v1/diagnostics", post(post_diagnostics))
        .route("/rcbilling/v1/subscribers/{id}/products", get(get_products))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            record_and_authorize,
        ))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            sign_responses,
        ))
        .with_state(state)
}

/// Records every request (method/path/headers/body) and enforces
/// `Authorization: Bearer <key>`, answering 401 `{"code": 7225}` otherwise —
/// the real backend's invalid-credentials behavior.
async fn record_and_authorize(
    State(state): State<Arc<ServerState>>,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 1 << 20).await.unwrap_or_default();

    let headers: BTreeMap<String, String> = RECORDED_HEADERS
        .iter()
        .filter_map(|name| {
            let value = parts.headers.get(*name)?.to_str().ok()?;
            Some(((*name).to_owned(), value.to_owned()))
        })
        .collect();
    let record = RecordedRequest {
        method: parts.method.to_string(),
        path: parts
            .uri
            .path_and_query()
            .map(|pq| pq.to_string())
            .unwrap_or_else(|| parts.uri.path().to_owned()),
        headers,
        body: serde_json::from_slice(&bytes).ok(),
    };
    if let Ok(mut requests) = state.requests.lock() {
        requests.push(record);
    }

    let authorized = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|key| !key.trim().is_empty());
    if !authorized {
        return backend_error(StatusCode::UNAUTHORIZED, 7225, "Invalid credentials error.");
    }

    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}

/// Signs responses on Trusted-Entitlements-verified endpoints exactly like
/// the real backend: adds `X-RevenueCat-Request-Time` and a 180-byte
/// `X-Signature` over `salt || api_key || nonce || path || post_params_hash
/// || request_time || etag || body`.
async fn sign_responses(
    State(state): State<Arc<ServerState>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();
    let api_key = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned();
    let nonce = header_string(request.headers(), "x-nonce");
    let post_params_hash = header_string(request.headers(), "x-post-params-hash");

    let response = next.run(request).await;
    let status = response.status();
    if !is_verified_path(&path) || !(status.is_success() || status == StatusCode::NOT_MODIFIED) {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let bytes = to_bytes(body, 1 << 20).await.unwrap_or_default();
    let etag = parts
        .headers
        .get(ETAG_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let request_time = Utc::now().timestamp_millis().to_string();

    let signature = state.signer.sign(
        &api_key,
        nonce.as_deref(),
        &path,
        post_params_hash.as_deref(),
        &request_time,
        etag.as_deref(),
        &bytes,
    );
    if let (Ok(sig), Ok(time)) = (signature.parse(), request_time.parse()) {
        parts.headers.insert("X-Signature", sig);
        parts.headers.insert("X-RevenueCat-Request-Time", time);
    }
    Response::from_parts(parts, Body::from(bytes))
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Endpoints with `supportsSignatureVerification` in the official SDKs.
fn is_verified_path(path: &str) -> bool {
    if path == "/v1/receipts"
        || path == "/v1/subscribers/identify"
        || path == "/v1/subscribers/redeem_purchase"
    {
        return true;
    }
    match path.strip_prefix("/v1/subscribers/") {
        Some(rest) => {
            !rest.contains('/')
                || rest.ends_with("/offerings")
                || rest.ends_with("/virtual_currencies")
        }
        None => false,
    }
}

/// Validates a client's `X-Post-Params-Hash` against the body fields it
/// claims to cover; a mismatch means the client hashed the wrong values.
fn validate_post_params_hash(headers: &HeaderMap, fields: &[(&str, &str)]) -> Option<Response> {
    let sent = headers.get("x-post-params-hash")?.to_str().ok()?;
    let expected = expected_post_params_hash(fields);
    if sent != expected {
        return Some(backend_error(
            StatusCode::BAD_REQUEST,
            7226,
            &format!("X-Post-Params-Hash mismatch: expected {expected}, got {sent}"),
        ));
    }
    None
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn get_subscriber(
    State(state): State<Arc<ServerState>>,
    Path(app_user_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    ensure_subscriber(&state, &app_user_id);
    let path = format!("/v1/subscribers/{app_user_id}");
    etag_response_for(
        &state,
        &path,
        &headers,
        subscriber_envelope(&state, &app_user_id),
    )
}

async fn get_offerings(
    State(state): State<Arc<ServerState>>,
    Path(app_user_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    ensure_subscriber(&state, &app_user_id);
    let offerings: Vec<Value> = state
        .offerings
        .iter()
        .map(|offering| {
            json!({
                "identifier": offering.identifier,
                "description": offering.description,
                "metadata": null,
                "packages": offering.packages.iter().map(|p| json!({
                    "identifier": p.identifier,
                    "platform_product_identifier": p.product_id,
                })).collect::<Vec<_>>(),
                // A dashboard-configured v1 template paywall, so demos and
                // tests can render one from the offering.
                "paywall": sample_paywall(offering),
            })
        })
        .collect();
    let body = json!({
        "current_offering_id": state.current_offering_id,
        "offerings": offerings,
    });
    let path = format!("/v1/subscribers/{app_user_id}/offerings");
    etag_response_for(&state, &path, &headers, body)
}

/// A v1 template paywall covering the offering's subscription packages, in
/// the exact `paywall` wire shape (template, config colors/features/packages,
/// localized strings).
fn sample_paywall(offering: &crate::state::MockOffering) -> Value {
    let package_ids: Vec<String> = offering
        .packages
        .iter()
        .map(|p| p.identifier.clone())
        .collect();
    let default_package = package_ids
        .iter()
        .find(|id| id.contains("annual"))
        .or_else(|| package_ids.first())
        .cloned();
    json!({
        "template_name": "2",
        "revision": 1,
        "default_locale": "en_US",
        "asset_base_url": "https://assets.pawwalls.com",
        "config": {
            "packages": package_ids,
            "default_package": default_package,
            "images": {},
            "colors": {
                "light": {
                    "background": "#fbf7f2",
                    "text_1": "#2b1d16",
                    "text_2": "#6b5b52",
                    "call_to_action_background": "#e0554d",
                    "call_to_action_foreground": "#ffffff",
                    "accent_1": "#e0554d"
                }
            },
            "display_restore_purchases": true,
            "tos_url": "https://example.com/tos",
            "privacy_url": "https://example.com/privacy"
        },
        "localized_strings": {
            "en_US": {
                "title": "Unlock Pro",
                "subtitle": "Everything, unlocked — cancel anytime.",
                "call_to_action": "Continue",
                "offer_details": "{{ total_price_and_per_month }}",
                "features": [
                    { "title": "Unlimited access", "icon_id": "lock" },
                    { "title": "Priority support", "icon_id": "chat" },
                    { "title": "No ads", "icon_id": "bell" }
                ]
            }
        }
    })
}

async fn get_products(
    State(state): State<Arc<ServerState>>,
    Path(app_user_id): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    ensure_subscriber(&state, &app_user_id);
    let requested: Vec<String> = query
        .unwrap_or_default()
        .split('&')
        .filter_map(|pair| pair.strip_prefix("id="))
        .map(percent_decode)
        .collect();

    let product_details: Vec<Value> = requested
        .iter()
        .filter_map(|id| state.product(id))
        .map(|product| {
            let price =
                json!({"amount_micros": product.price_micros, "currency": product.currency});
            let option = if product.product_type == "subscription" {
                json!({
                    "id": "base_option",
                    "price_id": format!("price_{}", product.identifier),
                    "base": {
                        "period_duration": product.period,
                        "cycle_count": 1,
                        "price": price,
                    },
                    "trial": null,
                    "intro_price": null,
                    "discount": null,
                })
            } else {
                json!({
                    "id": "base_option",
                    "price_id": format!("price_{}", product.identifier),
                    "base_price": price,
                    "discount": null,
                })
            };
            json!({
                "identifier": product.identifier,
                "product_type": product.product_type,
                "title": product.title,
                "description": product.description,
                "default_purchase_option_id": "base_option",
                "purchase_options": {"base_option": option},
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(json!({"product_details": product_details})),
    )
        .into_response()
}

async fn get_virtual_currencies(
    State(state): State<Arc<ServerState>>,
    Path(app_user_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    ensure_subscriber(&state, &app_user_id);
    let currencies: Map<String, Value> = state
        .virtual_currencies
        .iter()
        .map(|c| {
            (
                c.code.clone(),
                json!({"balance": c.balance, "code": c.code, "name": c.name}),
            )
        })
        .collect();
    let path = format!("/v1/subscribers/{app_user_id}/virtual_currencies");
    etag_response_for(
        &state,
        &path,
        &headers,
        json!({"virtual_currencies": currencies}),
    )
}

async fn post_receipts(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if state
        .receipt_rate_limits
        .fetch_update(
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
            |n| n.checked_sub(1),
        )
        .is_ok()
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("Retry-After", "0")],
            Json(json!({"code": 7101, "message": "Rate limited."})),
        )
            .into_response();
    }
    let fetch_token = body["fetch_token"].as_str().unwrap_or_default().to_owned();
    let app_user_id = body["app_user_id"].as_str().unwrap_or_default().to_owned();
    if let Some(error) = validate_post_params_hash(
        &headers,
        &[("app_user_id", &app_user_id), ("fetch_token", &fetch_token)],
    ) {
        return error;
    }
    let product_ids: Vec<String> = match &body["product_ids"] {
        Value::Array(ids) => ids
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_owned)
            .collect(),
        // purchases-js sends a singular `product_id` for the Test Store.
        _ => body["product_id"]
            .as_str()
            .map(|s| vec![s.to_owned()])
            .unwrap_or_default(),
    };

    if app_user_id.is_empty() {
        return backend_error(StatusCode::BAD_REQUEST, 7220, "Empty app user id.");
    }
    if !fetch_token.starts_with("test_") {
        // The real backend rejects tokens it cannot associate with a store.
        return backend_error(
            StatusCode::BAD_REQUEST,
            7103,
            "The receipt token is not valid.",
        );
    }
    let Some(product_id) = product_ids.first().cloned() else {
        return backend_error(StatusCode::BAD_REQUEST, 7662, "Malformed product IDs.");
    };
    let Some(product) = state.product(&product_id).cloned() else {
        return backend_error(
            StatusCode::BAD_REQUEST,
            7662,
            &format!("Unknown product '{product_id}'."),
        );
    };

    // Reject a token already redeemed by a DIFFERENT subscriber
    // (`BackendCannotTransferPurchase` -> ReceiptAlreadyInUseError).
    {
        let Ok(mut used) = state.used_tokens.lock() else {
            return backend_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                7110,
                "Internal server error.",
            );
        };
        match used.get(&fetch_token) {
            Some(owner) if *owner != app_user_id => {
                return backend_error(
                    StatusCode::BAD_REQUEST,
                    7102,
                    "The receipt is already in use by another subscriber.",
                );
            }
            _ => {
                used.insert(fetch_token.clone(), app_user_id.clone());
            }
        }
    }

    ensure_subscriber(&state, &app_user_id);
    grant_product(&state, &app_user_id, &product, &fetch_token);

    (
        StatusCode::OK,
        Json(subscriber_envelope(&state, &app_user_id)),
    )
        .into_response()
}

/// Records a purchase of `product` on the subscriber, shared by the receipts
/// and redemption endpoints.
fn grant_product(
    state: &ServerState,
    app_user_id: &str,
    product: &crate::state::MockProduct,
    store_transaction_id: &str,
) {
    let now = Utc::now();
    if let Ok(mut subscribers) = state.subscribers.lock() {
        if let Some(subscriber) = subscribers.get_mut(app_user_id) {
            if product.product_type == "subscription" {
                let original = subscriber
                    .subscriptions
                    .get(&product.identifier)
                    .map(|e| e.original_purchase_date)
                    .unwrap_or(now);
                subscriber.subscriptions.insert(
                    product.identifier.clone(),
                    SubscriptionEntry {
                        purchase_date: now,
                        original_purchase_date: original,
                        expires_date: expiry_for(product.period.as_deref(), now),
                        store_transaction_id: store_transaction_id.to_owned(),
                    },
                );
            } else {
                subscriber
                    .non_subscriptions
                    .entry(product.identifier.clone())
                    .or_default()
                    .push(NonSubscriptionEntry {
                        id: Uuid::new_v4().simple().to_string(),
                        purchase_date: now,
                        store_transaction_id: store_transaction_id.to_owned(),
                    });
            }
        }
    }
}

async fn post_identify(State(state): State<Arc<ServerState>>, Json(body): Json<Value>) -> Response {
    let old_id = body["app_user_id"].as_str().unwrap_or_default().to_owned();
    let new_id = body["new_app_user_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if new_id.is_empty() {
        return backend_error(StatusCode::BAD_REQUEST, 7220, "Empty app user id.");
    }
    ensure_subscriber(&state, &old_id);

    let created = {
        let Ok(mut subscribers) = state.subscribers.lock() else {
            return backend_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                7110,
                "Internal server error.",
            );
        };
        if subscribers.contains_key(&new_id) {
            false
        } else {
            // Alias semantics: the new identity inherits the anonymous
            // user's purchases, like the real identify/alias transfer.
            let inherited = subscribers.get(&old_id).cloned().unwrap_or_default();
            subscribers.insert(new_id.clone(), inherited);
            true
        }
    };

    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    (status, Json(subscriber_envelope(&state, &new_id))).into_response()
}

/// `POST /v1/subscribers/redeem_purchase`: body
/// `{"redemption_token", "app_user_id"}`. Success returns the subscriber
/// envelope; failures use the real backend codes 7849/7852/7853.
async fn post_redeem(State(state): State<Arc<ServerState>>, Json(body): Json<Value>) -> Response {
    let token = body["redemption_token"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let app_user_id = body["app_user_id"].as_str().unwrap_or_default().to_owned();
    if app_user_id.is_empty() {
        return backend_error(StatusCode::BAD_REQUEST, 7220, "Empty app user id.");
    }
    ensure_subscriber(&state, &app_user_id);

    let redemption = state
        .redemptions
        .lock()
        .ok()
        .and_then(|map| map.get(&token).cloned());
    let Some(redemption) = redemption else {
        return backend_error(
            StatusCode::BAD_REQUEST,
            7849,
            "Invalid Web Billing redemption token.",
        );
    };

    match redemption {
        MockRedemption::Expired { obfuscated_email } => (
            StatusCode::FORBIDDEN,
            Json(json!({
                "code": 7853,
                "message": "The link has expired.",
                "purchase_redemption_error_info": {"obfuscated_email": obfuscated_email},
            })),
        )
            .into_response(),
        MockRedemption::Valid { product_id } => {
            {
                let Ok(mut redeemed) = state.redeemed_by.lock() else {
                    return backend_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        7110,
                        "Internal server error.",
                    );
                };
                match redeemed.get(&token) {
                    Some(owner) if *owner != app_user_id => {
                        return backend_error(
                            StatusCode::FORBIDDEN,
                            7852,
                            "The purchase has already been redeemed.",
                        );
                    }
                    _ => {
                        redeemed.insert(token.clone(), app_user_id.clone());
                    }
                }
            }
            let Some(product) = state.product(&product_id).cloned() else {
                return backend_error(StatusCode::BAD_REQUEST, 7662, "Unknown product.");
            };
            grant_product(&state, &app_user_id, &product, &format!("web_{token}"));
            (
                StatusCode::OK,
                Json(subscriber_envelope(&state, &app_user_id)),
            )
                .into_response()
        }
    }
}

/// `POST /v1/diagnostics` — accepts `{"entries": [...]}` and records them.
async fn post_diagnostics(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<Value>,
) -> Response {
    let Some(entries) = body["entries"].as_array() else {
        return backend_error(StatusCode::BAD_REQUEST, 7226, "Missing entries.");
    };
    if let Ok(mut diagnostics) = state.diagnostics.lock() {
        diagnostics.extend(entries.iter().cloned());
    }
    (StatusCode::OK, Json(json!({}))).into_response()
}

const KNOWN_RESERVED_ATTRIBUTES: &[&str] = &[
    "$email",
    "$displayName",
    "$phoneNumber",
    "$pushToken",
    "$campaign",
    "$mediaSource",
];

async fn post_attributes(
    State(state): State<Arc<ServerState>>,
    Path(app_user_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    ensure_subscriber(&state, &app_user_id);
    let Some(attributes) = body["attributes"].as_object() else {
        return backend_error(StatusCode::BAD_REQUEST, 7264, "Invalid attributes body.");
    };

    let mut attribute_errors: Vec<Value> = Vec::new();
    for (key, entry) in attributes {
        let value = entry.get("value").cloned().unwrap_or(Value::Null);
        if key.starts_with('$') && !KNOWN_RESERVED_ATTRIBUTES.contains(&key.as_str()) {
            attribute_errors
                .push(json!({"key_name": key, "message": "Attribute key name is not valid."}));
        } else if key == "$email" {
            let valid = value
                .as_str()
                .map(|v| v.contains('@'))
                .unwrap_or(value.is_null());
            if !valid {
                attribute_errors.push(
                    json!({"key_name": key, "message": "Email address is not a valid email."}),
                );
            }
        }
    }
    if !attribute_errors.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "code": 7263,
                "message": "Some subscriber attributes keys were unable to be saved.",
                "attribute_errors": attribute_errors,
            })),
        )
            .into_response();
    }

    if let Ok(mut subscribers) = state.subscribers.lock() {
        if let Some(subscriber) = subscribers.get_mut(&app_user_id) {
            for (key, entry) in attributes {
                subscriber.attributes.insert(
                    key.clone(),
                    entry.get("value").cloned().unwrap_or(Value::Null),
                );
            }
        }
    }
    (StatusCode::OK, Json(json!({}))).into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ensure_subscriber(state: &ServerState, app_user_id: &str) {
    if app_user_id.is_empty() {
        return;
    }
    if let Ok(mut subscribers) = state.subscribers.lock() {
        subscribers
            .entry(app_user_id.to_owned())
            .or_default()
            .first_seen
            .get_or_insert(Utc::now());
    }
}

/// Builds the `{"request_date", "request_date_ms", "subscriber": {...}}`
/// envelope with the exact field names of the real API.
fn subscriber_envelope(state: &ServerState, app_user_id: &str) -> Value {
    let now = Utc::now();
    let subscriber = state.subscriber(app_user_id).unwrap_or_default();
    let first_seen = subscriber.first_seen.unwrap_or(now);

    let subscriptions: Map<String, Value> = subscriber
        .subscriptions
        .iter()
        .map(|(product_id, entry)| {
            (
                product_id.clone(),
                json!({
                    "auto_resume_date": null,
                    "billing_issues_detected_at": null,
                    "expires_date": entry.expires_date.map(iso),
                    "grace_period_expires_date": null,
                    "is_sandbox": true,
                    "original_purchase_date": iso(entry.original_purchase_date),
                    "period_type": "normal",
                    "purchase_date": iso(entry.purchase_date),
                    "refunded_at": null,
                    "store": "test_store",
                    "store_transaction_id": entry.store_transaction_id,
                    "unsubscribe_detected_at": null,
                    "ownership_type": "PURCHASED",
                }),
            )
        })
        .collect();

    let non_subscriptions: Map<String, Value> = subscriber
        .non_subscriptions
        .iter()
        .map(|(product_id, entries)| {
            (
                product_id.clone(),
                Value::Array(
                    entries
                        .iter()
                        .map(|e| {
                            json!({
                                "id": e.id,
                                "is_sandbox": true,
                                "original_purchase_date": iso(e.purchase_date),
                                "purchase_date": iso(e.purchase_date),
                                "store": "test_store",
                                "store_transaction_id": e.store_transaction_id,
                            })
                        })
                        .collect(),
                ),
            )
        })
        .collect();

    // Entitlements derive from purchases at request time: the latest expiry
    // per entitlement across granting products.
    let mut entitlements: Map<String, Value> = Map::new();
    for product in &state.products {
        let Some(entitlement_id) = &product.entitlement_id else {
            continue;
        };
        if let Some(entry) = subscriber.subscriptions.get(&product.identifier) {
            let candidate = json!({
                "expires_date": entry.expires_date.map(iso),
                "grace_period_expires_date": null,
                "product_identifier": product.identifier,
                "purchase_date": iso(entry.purchase_date),
            });
            let replace = match entitlements.get(entitlement_id) {
                Some(existing) => {
                    existing
                        .get("expires_date")
                        .map(|d| d.is_null())
                        .unwrap_or(true)
                        || existing["expires_date"].as_str() < candidate["expires_date"].as_str()
                }
                None => true,
            };
            if replace {
                entitlements.insert(entitlement_id.clone(), candidate);
            }
        }
        if subscriber
            .non_subscriptions
            .contains_key(&product.identifier)
        {
            entitlements.insert(
                entitlement_id.clone(),
                json!({
                    "expires_date": null,
                    "grace_period_expires_date": null,
                    "product_identifier": product.identifier,
                    "purchase_date": iso(now),
                }),
            );
        }
    }

    json!({
        "request_date": iso(now),
        "request_date_ms": now.timestamp_millis(),
        "subscriber": {
            "entitlements": entitlements,
            "first_seen": iso(first_seen),
            "last_seen": iso(now),
            "management_url": null,
            "non_subscriptions": non_subscriptions,
            "original_app_user_id": app_user_id,
            "original_application_version": "1.0",
            "original_purchase_date": null,
            "other_purchases": {},
            "subscriptions": subscriptions,
        }
    })
}

/// Non-fractional Z-suffixed ISO 8601, the format the SDKs emit and accept.
fn iso(date: chrono::DateTime<Utc>) -> String {
    date.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn backend_error(status: StatusCode, code: i64, message: &str) -> Response {
    (status, Json(json!({"code": code, "message": message}))).into_response()
}

/// RevenueCat's custom ETag protocol: compare the request's
/// `X-RevenueCat-ETag` with the hash of the fresh body; matching -> 304.
/// A `force_304_next` mark answers 304 regardless, exercising the client's
/// 304-without-cache retry.
fn etag_response_for(
    state: &ServerState,
    path: &str,
    headers: &HeaderMap,
    body: Value,
) -> Response {
    let text = body.to_string();
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    let etag = format!("{:x}", hasher.finish());

    let forced = state
        .force_not_modified
        .lock()
        .map(|mut set| set.remove(path))
        .unwrap_or(false);
    let request_etag = headers
        .get(ETAG_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if forced || (!request_etag.is_empty() && request_etag == etag) {
        return (StatusCode::NOT_MODIFIED, [(ETAG_HEADER, etag)]).into_response();
    }
    (StatusCode::OK, [(ETAG_HEADER, etag)], Json(body)).into_response()
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(byte) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_reverses_client_encoding() {
        assert_eq!(
            percent_decode("%24RCAnonymousID%3Aabc"),
            "$RCAnonymousID:abc"
        );
        assert_eq!(percent_decode("coins%5F100"), "coins_100");
        assert_eq!(percent_decode("plain"), "plain");
        // Malformed sequences fall through untouched.
        assert_eq!(percent_decode("bad%2"), "bad%2");
    }

    #[test]
    fn verified_paths_match_the_official_endpoint_set() {
        for path in [
            "/v1/receipts",
            "/v1/subscribers/identify",
            "/v1/subscribers/redeem_purchase",
            "/v1/subscribers/gon",
            "/v1/subscribers/gon/offerings",
            "/v1/subscribers/gon/virtual_currencies",
        ] {
            assert!(is_verified_path(path), "{path} must be signed");
        }
        for path in [
            "/v1/subscribers/gon/attributes",
            "/v1/diagnostics",
            "/rcbilling/v1/subscribers/gon/products",
        ] {
            assert!(!is_verified_path(path), "{path} must not be signed");
        }
    }

    #[test]
    fn post_params_hash_validation_rejects_mismatches() {
        // Arrange: a header hashing DIFFERENT values than the body claims.
        let mut headers = HeaderMap::new();
        let wrong = expected_post_params_hash(&[("app_user_id", "other")]);
        headers.insert("x-post-params-hash", wrong.parse().unwrap());

        // Act / Assert
        assert!(validate_post_params_hash(&headers, &[("app_user_id", "gon")]).is_some());
        let right = expected_post_params_hash(&[("app_user_id", "gon")]);
        headers.insert("x-post-params-hash", right.parse().unwrap());
        assert!(validate_post_params_hash(&headers, &[("app_user_id", "gon")]).is_none());
        // Absent header (verification disabled client) is accepted.
        assert!(validate_post_params_hash(&HeaderMap::new(), &[("k", "v")]).is_none());
    }
}
