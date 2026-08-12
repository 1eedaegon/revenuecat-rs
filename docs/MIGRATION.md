# Migration guide

Why this exists: the official RevenueCat SDKs ship migration guides for two
reasons, and both map onto this crate.

1. **Major versions rename or remove public API.** The iOS SDK's v3→v4 bump
   renamed `PurchaserInfo` → `CustomerInfo` (and the `Purchases` module →
   `RevenueCat`); v4→v5 made StoreKit 2 the default and removed
   `usesStoreKit2IfAvailable`. Every such bump ships an old→new mapping table.
2. **People migrate *to* RevenueCat** from raw StoreKit / Play Billing or
   another IAP vendor, so the guides also cover observer mode and importing
   existing subscribers.

This document is the `revenuecat-rs` equivalent: how to come from an official
SDK, from raw REST calls, and how breaking changes are handled here.

## Versioning policy

`revenuecat-rs` follows SemVer from 1.0: breaking changes bump **major**,
additive changes bump **minor**, fixes bump **patch**. Pin a major range and
read the release notes before a major upgrade:

```toml
revenuecat-rs = "1"   # 1.x, additive-only until 2.0
```

Breaking changes are called out in the GitHub release for each tag. The public
API is what's re-exported from the crate root (`revenuecat::…`); anything not
re-exported is internal and may change without notice.

## Coming from the official SDKs

Two differences dominate; the rest is a near-mechanical rename.

### 1. Instance-based, not a global singleton

The native SDKs expose a process-wide singleton after `configure`
(`Purchases.shared` / `Purchases.sharedInstance`). `revenuecat-rs` returns the
instance and lets **you** own it: configure once and manage it (in a Tauri
app, put it in state and borrow it in commands):

```rust
// Official SDKs:  Purchases.configure(...);  then  Purchases.shared.getOfferings()
// revenuecat-rs:  own the value you get back.
let purchases = revenuecat::Purchases::configure(
    revenuecat::Configuration::builder("test_KEY").app_user_id("gon").build()?,
)?;
let offerings = purchases.get_offerings().await?;
```

No hidden global state, no `configure`-before-use ordering traps: the type
system enforces that you have a configured instance.

### 2. You already speak the *modern* vocabulary

This crate mirrors the official SDKs' **current** wire format, so the renames
you'd hit migrating an old native app are already done: it's `CustomerInfo`
(never `PurchaserInfo`), `get_offerings` (never `offerings`), `restore_purchases`
(never `restoreTransactions`).

| Concept | purchases-ios (Swift) | purchases-android (Kotlin) | `revenuecat-rs` |
|---|---|---|---|
| Configure | `Purchases.configure(withAPIKey:)` | `Purchases.configure(...)` | `Purchases::configure(Configuration)` |
| Current user | `Purchases.shared.appUserID` | `Purchases.sharedInstance.appUserID` | `purchases.app_user_id()` |
| Customer info | `getCustomerInfo()` | `getCustomerInfo()` | `purchases.get_customer_info(policy)` |
| Invalidate cache | `invalidateCustomerInfoCache()` | `invalidateCustomerInfoCache()` | `purchases.invalidate_customer_info_cache()` |
| Offerings | `getOfferings()` | `getOfferings()` | `purchases.get_offerings()` |
| Products | `getProducts(_:)` | `getProducts(...)` | `purchases.get_products(&ids)` |
| Purchase package | `purchase(package:)` | `purchase(PurchaseParams)` | `purchases.purchase_package(pkg)` |
| Purchase product | `purchase(product:)` | `purchase(PurchaseParams)` | `purchases.purchase_product(prod)` |
| Restore | `restorePurchases()` | `restorePurchases()` | `purchases.restore_purchases()` |
| Identify | `logIn(_:)` | `logIn(...)` | `purchases.log_in(id)` → `(info, created)` |
| Sign out | `logOut()` | `logOut()` | `purchases.log_out()` |
| Set email | `attribution.setEmail(_:)` | `setEmail(...)` | `purchases.set_email(email)` |
| Set attributes | `attribution.setAttributes(_:)` | `setAttributes(...)` | `purchases.set_attributes(map)` |
| Web redemption | `parseAsWebPurchaseRedemption` | `parseAsWebPurchaseRedemption` | `Purchases::parse_web_purchase_redemption(url)` |
| Redeem | `redeemWebPurchase(_:)` | `redeemWebPurchase(...)` | `purchases.redeem_web_purchase(&link)` |
| Virtual currencies | `virtualCurrencies()` | `getVirtualCurrencies()` | `purchases.get_virtual_currencies()` |

Callbacks/delegates become `async fn` returning `Result`. The customer-info
delegate/stream maps to `purchases.set_customer_info_listener(...)`.

### 3. The store: `StoreBilling` instead of a bundled StoreKit/Billing layer

The native SDKs bundle StoreKit / Play Billing. Here the store is a trait you
supply: see the [`StoreBilling`](../README.md#real-stores-the-storebilling-trait)
section. Consequences that mirror the official SDKs:

- **StoreKit version.** `tauri-plugin-revenuecat` implements `StoreBilling` over
  **StoreKit 2** (iOS 15+/macOS 12+) and Play Billing. This is the same reason
  RevenueCat's own v5 leans on StoreKit 2, the modern, Swift-concurrency API.
- **"Observer mode" analog.** RevenueCat renamed observer mode to
  `purchasesAreCompletedBy`. The equivalent here is simply *your* `StoreBilling`
  impl: you own the store transaction and decide when to
  `finish_transaction(tx, consume)`. With a `test_` key and the built-in Test
  Store, purchases are finished for you.
- **Importing existing subscribers** is unchanged and server-side: call
  `restore_purchases()` (→ `POST /v1/receipts`) once per user, or do a backend
  receipt/token import from the RevenueCat dashboard. The client SDK does not
  replay history on every launch.

## Coming from raw REST calls

If you currently call the RevenueCat v1 API by hand (reqwest/curl), this crate
replaces that with typed models, ETag caching, retry/backoff, and Trusted
Entitlements verification. Endpoint → method map:

| Endpoint | `revenuecat-rs` |
|---|---|
| `GET /v1/subscribers/{id}` | `get_customer_info(policy)` |
| `GET /v1/subscribers/{id}/offerings` | `get_offerings()` (incl. `offering.paywall`) |
| `GET /rcbilling/v1/subscribers/{id}/products` | `get_products(&ids)` |
| `POST /v1/receipts` | `purchase_package` / `purchase_product` / `restore_purchases` |
| `POST /v1/subscribers/identify` | `log_in(id)` |
| `POST /v1/subscribers/{id}/attributes` | `set_attributes` / `set_email` / `set_display_name` |
| `GET /v1/subscribers/{id}/virtual_currencies` | `get_virtual_currencies()` |
| `POST /v1/subscribers/redeem_purchase` | `redeem_web_purchase(&link)` |
| `POST /v1/diagnostics` | `flush_diagnostics()` (opt-in) |

Keep your existing RevenueCat **dashboard** setup (offerings, entitlements,
products). This is a client for the same backend, not a replacement for it.

## See also

- [Supported platforms & requirements](../README.md#supported-platforms--requirements)
- [`StoreBilling` trait](../README.md#real-stores-the-storebilling-trait)
