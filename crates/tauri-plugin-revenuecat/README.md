# tauri-plugin-revenuecat

[![npm](https://img.shields.io/npm/v/tauri-plugin-revenuecat)](https://www.npmjs.com/package/tauri-plugin-revenuecat)
[![crates.io](https://img.shields.io/crates/v/tauri-plugin-revenuecat)](https://crates.io/crates/tauri-plugin-revenuecat)

Tauri 2.12.4+ mobile plugin that bridges **StoreKit 2** (iOS) and **Play Billing**
(Android) into the [`revenuecat-rs`](https://crates.io/crates/revenuecat-rs)
crate's `StoreBilling` seam, and exposes the SDK to the webview as a typed
TypeScript package. It is a feature-scoped direct port against the reviewed
RevenueCat baselines, not a claim of complete native SDK or UI parity.

For the full SDK surface, model reference, and desktop story, see the
[project README](https://github.com/1eedaegon/revenuecat-rs#readme).

## Install

TypeScript-owned SDK state (run from the Tauri app root):

```sh
npm install tauri-plugin-revenuecat && cargo add --manifest-path src-tauri/Cargo.toml tauri-plugin-revenuecat
```

Rust-owned SDK state:

```sh
cargo add --manifest-path src-tauri/Cargo.toml tauri-plugin-revenuecat revenuecat-rs
```

## Register

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_revenuecat::init())
    // ... your other setup
```

Grant the commands in `src-tauri/capabilities/default.json`:

```json
{ "identifier": "default", "windows": ["main"], "permissions": ["revenuecat:default"] }
```

Run on desktop, Android, or iOS:

```sh
npm run tauri dev
npm run tauri android dev
npm run tauri ios dev
```

## Two tracks

Both register the plugin the same way; they differ in **who owns the SDK**.

### Track 1 — TypeScript (the plugin owns the SDK)

Drive everything from the webview, no per-app Rust glue:

```ts
import { configure, getOfferings, purchasePackage } from "tauri-plugin-revenuecat";

// appl_/goog_ keys wire the native store automatically on mobile.
await configure({ apiKey: "test_YOUR_KEY", appUserId: "gon" });

const offerings = await getOfferings();          // typed: Offerings
const pkg = offerings.current?.packages[0];
if (pkg) {
  const result = await purchasePackage(pkg.identifier);
  // result.customer_info.entitlements.all.pro?.is_active
}
```

Subscribe to customer-info changes (the `updatedCustomerInfoListener` equivalent):

```ts
import { onCustomerInfoUpdated } from "tauri-plugin-revenuecat";

const unlisten = await onCustomerInfoUpdated((info) => {
  setPro(info.entitlements.all.pro?.is_active ?? false);
});
```

### Track 2 — Rust (you own the SDK)

Keep the SDK logic in Rust; the plugin supplies the native store on mobile via
`store_billing`:

```rust
.setup(|app| {
    let mut builder = revenuecat::Configuration::builder("appl_or_test_KEY");
    // Native store on mobile; Err on desktop (a `test_` key needs no store).
    if let Ok(billing) = tauri_plugin_revenuecat::store_billing(app.handle()) {
        builder = builder.store_billing(billing);
    }
    app.manage(revenuecat::Purchases::configure(builder.build()?)?);
    Ok(())
})
```

## TypeScript API

`configure`, `getOfferings`, `getCustomerInfo`, `purchasePackage`, `restore`,
`logIn`, `logOut`, `setEmail`, `sessionInfo`, `manageSubscriptions` (the store's
manage/cancel URL), and the `onCustomerInfoUpdated` listener. Model types
(`Offerings`, `CustomerInfo`, `Paywall`, …) ship with the package.

## License

MIT
