# revenuecat-rs

Unofficial **RevenueCat SDK for Rust**, protocol-compatible with the official
[purchases-ios], [purchases-android], and [purchases-js] SDKs (all MIT). Wire
formats were extracted from their sources and test fixtures.

A `test_` (Test Store) key needs **no native store** — purchases are simulated
end to end, so the SDK, its tests, and the Tauri demo all run on desktop and
CI. Real stores plug in through one trait.

![Tauri demo](docs/demo-ui.png)

```toml
[dependencies]
revenuecat-rs = "0.3"   # imported in code as `revenuecat`
```

## Usage

### Configure

```rust
use revenuecat::{Configuration, Purchases};

// Test Store — real backend, no native store, no store account.
let purchases = Purchases::configure(
    Configuration::builder("test_YOUR_KEY")
        .app_user_id("gon")          // omit for an anonymous $RCAnonymousID:…
        .build()?,
)?;

// Point at a mock / staging host (same idea as `Purchases.proxyURL`).
let staging = Configuration::builder("test_KEY")
    .proxy_url("http://127.0.0.1:8000")
    .build()?;
```

### Offerings and purchases

```rust
let offerings = purchases.get_offerings().await?;
let monthly = offerings.current().and_then(|o| o.monthly()).expect("package");
println!("{} — {}", monthly.store_product.title, monthly.store_product.price.formatted());

// store flow -> POST /v1/receipts -> server-computed CustomerInfo
let result = purchases.purchase_package(monthly).await?;
assert!(result.customer_info.entitlements.is_active("pro"));
```

### Customer info and entitlements

```rust
use revenuecat::CacheFetchPolicy;

let info = purchases.get_customer_info(CacheFetchPolicy::default()).await?;
if let Some(pro) = info.entitlements.get("pro") {
    println!("active={} renews={} until={:?}", pro.is_active, pro.will_renew, pro.expiration_date);
}
println!("active subs: {:?}", info.active_subscriptions());
```

### Identity and attributes

```rust
let (info, created) = purchases.log_in("gon-pro").await?;   // alias & switch user
purchases.set_email("gon@example.com").await?;              // reserved $email attribute
purchases.set_attributes([("plan_intent".into(), Some("pro".into()))].into()).await?;
purchases.log_out().await?;                                 // back to a fresh anonymous id
```

### Trusted Entitlements (Ed25519 signatures)

Off by default, like the official SDKs.

```rust
use revenuecat::EntitlementVerificationMode;

let config = Configuration::builder("test_KEY")
    // Informational: verify + report. Enforced: failures become errors.
    .entitlement_verification_mode(EntitlementVerificationMode::Informational)
    .build()?;

let info = purchases.get_customer_info(Default::default()).await?;
assert!(info.entitlements.verification.is_verified());
```

Verified endpoints send `X-Nonce`; signed POSTs add `X-Post-Params-Hash`.
Against `revenuecat-mock` (its own test chain) add
`.verification_root_key(revenuecat_mock::test_root_public_key_b64())`.

### Web purchase redemption

```rust
if let Some(link) = Purchases::parse_web_purchase_redemption(&deep_link_url) {
    match purchases.redeem_web_purchase(&link).await? {
        RedeemResult::Success { customer_info } => { /* granted */ }
        RedeemResult::Expired { obfuscated_email } => { /* resend to email */ }
        other => { /* InvalidToken / PurchaseBelongsToOtherUser / Error */ }
    }
}
```

### Virtual currencies and diagnostics

```rust
let balances = purchases.get_virtual_currencies().await?;
println!("gold: {}", balances.balance("GLD"));

// Diagnostics are opt-in; flush batches to the diagnostics host.
let purchases = Purchases::configure(
    Configuration::builder("test_KEY").diagnostics_enabled(true).build()?,
)?;
purchases.flush_diagnostics().await?;
```

### Real stores — the `StoreBilling` trait

Everything above the store is pure logic + HTTP. Implement one trait to plug
in StoreKit / Play Billing and pass it via `ConfigurationBuilder::store_billing`:

```rust
#[async_trait::async_trait]
impl revenuecat::StoreBilling for MyStoreBridge {
    async fn query_products(&self, ids: &[String]) -> revenuecat::Result<Vec<StoreProduct>>;
    async fn purchase(&self, product: &StoreProduct) -> revenuecat::Result<StoreTransaction>;
    async fn query_purchases(&self) -> revenuecat::Result<Vec<StoreTransaction>>;
    async fn finish_transaction(&self, tx: &StoreTransaction, consume: bool) -> revenuecat::Result<()>;
}

let purchases = Purchases::configure(
    Configuration::builder("appl_YOUR_KEY").store_billing(billing).build()?,
)?;
```

`crates/tauri-plugin-revenuecat` implements this over StoreKit 2 (Swift) and
Play Billing (Kotlin) for Tauri mobile apps.

### In a Tauri app

The SDK is instance-based — configure once, manage it, borrow in commands.
Every model and `revenuecat::Error` are `Serialize`, so commands return them
directly (with a stable error `code` for the UI).

```rust
struct AppState { purchases: revenuecat::Purchases }

#[tauri::command]
async fn purchase(
    state: tauri::State<'_, AppState>,
    package_id: String,   // Tauri maps JS `packageId` -> Rust `package_id`
) -> Result<revenuecat::PurchaseResult, revenuecat::Error> {
    let offerings = state.purchases.get_offerings().await?;
    let package = offerings.current().and_then(|o| o.package(&package_id)).unwrap();
    state.purchases.purchase_package(package).await
}

tauri::Builder::default()
    .setup(|app| {
        let purchases = revenuecat::Purchases::configure(/* … */)?;
        tauri::Manager::manage(app, AppState { purchases });
        Ok(())
    })
    .invoke_handler(tauri::generate_handler![purchase /*, … */]);
```

Test commands headlessly with `tauri::test::mock_builder` + `get_ipc_response`
(see `demo/tauri-app/src-tauri/tests/commands.rs`). Note the ACL local origin
is `tauri://localhost` on macOS/Linux but `http://tauri.localhost` on Windows.

## Workspace

| Crate | What it is |
|---|---|
| `crates/revenuecat` | The SDK: models, HTTP client, backend ops, `Purchases` facade, `StoreBilling` trait + simulated Test Store |
| `crates/revenuecat-mock` | In-process mock of the RevenueCat API (axum), signs responses with a test Ed25519 chain |
| `crates/tauri-plugin-revenuecat` | Tauri 2 mobile plugin: StoreKit 2 / Play Billing shims behind `StoreBilling` |
| `demo/tauri-app` | Tauri 2 demo — configured at runtime from a key input |

## Demo

```sh
cargo run -p revenuecat-tauri-demo
```

Enter an API key on the setup screen to pick the backend:

- **empty** → embedded mock backend, fully offline;
- `test_…` → real RevenueCat backend, simulated Test Store (no store account);
- `appl_…` / `goog_…` → StoreKit 2 / Play Billing (mobile, `--features native-store`).

There's also a CLI walkthrough of the same flow:

```sh
cargo run -p revenuecat-rs --example test_store_flow
```

## Testing

```sh
cargo test --workspace          # unit + integration + Tauri IPC (110+ tests)
cargo clippy --workspace --all-targets   # unwrap is denied in lib code
cargo deny check                # advisories / licenses
```

The integration suite runs the real SDK stack against `revenuecat-mock` and
asserts exact wire behavior — request paths, headers, receipt-body fields,
ETag 304 replay, and full Ed25519 verification.

## Protocol coverage

| Endpoint | Status |
|---|---|
| `GET /v1/subscribers/{id}` | ✅ custom ETag caching |
| `POST /v1/receipts` | ✅ android-shape body, 429 retry w/ backoff |
| `GET /v1/subscribers/{id}/offerings` | ✅ |
| `GET /rcbilling/v1/subscribers/{id}/products` | ✅ Test Store products |
| `POST /v1/subscribers/identify` | ✅ created = HTTP 201 |
| `POST /v1/subscribers/{id}/attributes` | ✅ incl. `attribute_errors` |
| `GET /v1/subscribers/{id}/virtual_currencies` | ✅ cache + invalidation |
| Trusted Entitlements (`X-Signature`, Ed25519) | ✅ root → intermediate → payload; nonce + params-hash; 3 modes |
| `POST /v1/subscribers/redeem_purchase` | ✅ typed results + deep-link parser |
| `POST /v1/diagnostics` | ✅ opt-in; Android entry shape and retry semantics |
| Native stores (StoreKit 2 / Play Billing) | 🔶 `tauri-plugin-revenuecat` shims code-complete; device E2E pending |
| Paywalls / customer center UI | ❌ out of scope |

Not affiliated with RevenueCat. Only the documented surface plus the endpoints
the official MIT-licensed clients use is spoken.

## License

[MIT](LICENSE).

[purchases-ios]: https://github.com/RevenueCat/purchases-ios
[purchases-android]: https://github.com/RevenueCat/purchases-android
[purchases-js]: https://github.com/RevenueCat/purchases-js
