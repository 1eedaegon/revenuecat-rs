# RevenueCat compatibility policy

`revenuecat-rs` is a direct, independent port of RevenueCat client behavior.
It does not link the official Apple, Android, or JavaScript SDKs. The upstream
versions in [`upstream/versions.json`](../upstream/versions.json) are the last
versions reviewed for protocol and purchase-lifecycle changes; they are not
package dependencies and do not imply UI feature parity.

**Last reviewed:** 2026-11-05. Compatibility is feature-by-feature, not a claim
of perfect parity with any official SDK.

## Compatibility contract

The project tracks three distinct layers:

1. **Backend wire compatibility** — request paths, headers, bodies, response
   parsing, caching, diagnostics, and Trusted Entitlements.
2. **Store lifecycle compatibility** — StoreKit 2 and Play Billing product
   lookup, purchase, restore, acknowledgement, and transaction recovery.
3. **Public API compatibility** — existing Rust and TypeScript consumers should
   continue compiling throughout the 1.x line.

Unknown response fields and enum values must remain non-fatal. New upstream
data should first be available through raw data or additive accessors. A new
required field on a public struct, a new required `StoreBilling` method, or a
new variant on an exhaustively matchable public enum requires a major release.
Provided trait methods, builders, new wrapper types, and optional IPC fields
are suitable for a 1.x release.

`Offering`/`Package` values produced by the SDK belong to the identity and
snapshot that loaded them. Identity changes invalidate their store-plan
selection, so stale SDK-issued packages fail before native checkout and callers
must fetch offerings again. Manually constructed 1.x packages retain their
planless compatibility path.

## Reviewed upstreams

| Upstream | Reviewed version | Role in this port |
|---|---:|---|
| [`purchases-ios`](https://github.com/RevenueCat/purchases-ios/releases/tag/5.102.1) | 5.102.1 | StoreKit lifecycle, Apple transaction and receipt behavior |
| [`purchases-android`](https://github.com/RevenueCat/purchases-android/releases/tag/11.1.0) | 11.1.0 | Canonical backend request shapes, Play Billing lifecycle, public model semantics |
| [`purchases-js`](https://github.com/RevenueCat/purchases-js/releases/tag/1.74.0) | 1.74.0 | Test Store/Web Billing product shapes and cross-platform fixtures |
| [Google Play Billing Library](https://developer.android.com/google/play/billing/release-notes) | 9.1.0 | Android store binary used by the direct Kotlin bridge |

The weekly `Upstream watch` workflow compares all source paths that can affect
the direct port. Updating the versions file means the diff was reviewed; each
new capability still needs an explicit implementation and conformance test.

## Language and platform compatibility

| Layer | Compatibility floor / build baseline | Policy |
|---|---|---|
| Rust | Rust 1.90, edition 2021 | Keep the declared MSRV; new public behavior is additive throughout 1.x |
| TypeScript | TypeScript 5.4+, Tauri API 2.12+ | Preserve existing command names and option fields; new options are optional |
| iOS / iPadOS | Swift tools 5.5, iOS 15+ | StoreKit 2 direct bridge; no dependency on `purchases-ios` |
| Android | API 24+, Java 17, compile SDK 36 | Play Billing 9.1.0 direct bridge; consumer target SDK remains app-controlled |
| macOS desktop | Rust core only | Test Store and Web Billing work; the native StoreKit purchase bridge is not wired into Tauri on macOS |

The Android bridge uses the Java `billing` artifact rather than `billing-ktx`,
so Play Billing 9.1.0 does not force Kotlin 2.1 metadata into the consuming
Tauri template. The Swift package is intentionally iOS-only; macOS support is
limited to the Rust core's Test Store and Web Billing paths.

## Ownership and recovery paths

The two supported Tauri integration tracks have different lifecycle duties.

### Standard TypeScript / guest-JS track

The plugin-owned `configure()` path persists the effective `appUserId` in the
app data directory. If no ID is supplied, it reuses the persisted ID or creates
one anonymous ID. `logIn` and `logOut` first persist a current→pending journal,
then settle it only after the backend and local identity switch succeed. A
crash or disk-write failure is resumed on the next configure; malformed or
oversized identity files fail closed instead of silently creating a new user.

For a real-store session, guest JS reconciles at configure time, window resume,
window focus, and native `transaction-updated` events. Android configure uses a
full active-purchase query so an already-acknowledged Play token is recovered
after reinstall/local-state loss; later lifecycle events use the pending queue.
The direct port does not yet mirror Android's persistent sent-token cache, so
the backend may receive an idempotent repost of active Play receipts once per
app process in exchange for fail-safe recovery. iOS configure and lifecycle
events use unfinished StoreKit transactions. A new purchase waits for
startup/retry reconciliation to succeed, and the Rust core
independently blocks reopening store UI after a native transaction until
recovery completes.

New native purchases carry store-side account attribution without exposing the
raw App User ID to the store: Android uses the same SHA-256/Base64
`obfuscatedAccountId` policy as purchases-android 11.1, while StoreKit receives
`appAccountToken` only when the App User ID is already UUID-shaped.

### Rust-owned `StoreBilling` track

`tauri_plugin_revenuecat::store_billing()` supplies the native bridge only. It
does not install the guest-JS identity storage or wire native transaction
events into a Rust-owned `Purchases` instance. The host application must:

1. durably store and pass its `app_user_id` on the next launch, including the
   current value after `log_in` or `log_out`; and
2. call `sync_pending_purchases()` after startup and whenever the application
   returns to the foreground (or in an equivalent native-event/polling loop).
3. after any native purchase error, call `sync_pending_purchases()` before an
   identity change or another purchase. The store may have created a
   transaction even when journal persistence or the IPC response failed, so
   the Rust core deliberately treats every native error as ambiguous.

Use `sync_purchases()` as an explicit one-time/full recovery when migrating an
older integration, repairing lost local state, or intentionally scanning all
transactions returned by the platform's full query. It is broader than the
normal pending queue and should not replace the pending-only lifecycle path. A
user-triggered `restore_purchases()` remains the explicit account-sharing/restore
operation.

## Current scope

| Capability | Status |
|---|---|
| Customer info, offerings, identity, subscriber attributes | Implemented |
| Receipt posting and restore | Implemented; platform product context is preserved where the store exposes it |
| Test Store and Web Billing product parsing | Implemented |
| Trusted Entitlements | Implemented |
| StoreKit 2 purchases | Implemented for iOS/iPadOS 15+; native App Store sandbox/production device E2E remains unrun |
| Play Billing purchases | Implemented with Billing 9.1.0 for Android API 24+; native Play test/production device E2E remains unrun |
| macOS native StoreKit purchases | Unsupported by the Tauri bridge; use Test Store or Web Billing |
| Legacy paywall configuration | Parsed; rendering belongs to the application |
| Component paywalls | Configuration is exposed as data; no native RevenueCatUI renderer(presstar or buy the maintainer a coffee.) |
| Multi-step workflows, native Customer Center UI, IAM, ad rewards | Not implemented |

The existing physical-device checks used RevenueCat's simulated Test Store.
They prove the mobile Tauri/HTTP path, not a StoreKit or Play Billing purchase.

### Android correlation limit

Before launching checkout, the Android bridge queries owned purchases only for
the product's Play type (`SUBS` or `INAPP`). Unlike purchases-android 11.1,
which can continue with an obfuscated account ID after its subscription lookup
fails, this bridge deliberately fails closed: a complete same-type snapshot is
required to keep an older Play token from consuming a newer purchase callback.

The Android bridge serializes purchases initiated by this plugin and remembers
known Play purchase tokens. A Play purchase callback still does not carry its
selected base plan. Mirroring purchases-android 11.1, the Rust core records
plans launched in the process and, after receipt posting, compares the exact
Play order ID plus the backend-confirmed `product_plan_identifier`. If Play
delivers a previously launched different plan under the current callback, the
SDK refreshes `CustomerInfo`, finishes the accepted transaction, and returns
`ProductAlreadyPurchasedError` instead of reporting the wrong plan as the
current purchase.

A token created outside the current process has no local launch history, so
access must still come from backend-validated `CustomerInfo`/entitlements,
never from the locally selected offer or callback alone.

## Webview trust boundary

The existing 1.x `PurchaseResult.transaction.purchase_token` contract exposes
the Play purchase token or StoreKit JWS to the calling webview. Removing it
would be a breaking change, so applications must treat the Tauri webview as a
trusted payment surface: enforce a strict CSP, do not load untrusted scripts,
and grant the RevenueCat command permission only to the purchase window. Access
must always be decided from backend-validated `CustomerInfo`, never by decoding
or trusting that token in JavaScript.

## Test Store release guard

Mobile release builds reject `test_` keys by default because simulated
purchases can grant test entitlements. Controlled release-mode testing can opt
in explicitly with
`dangerously_allow_test_store_in_release_builds(true)` in Rust or
`dangerouslyAllowTestStoreInReleaseBuilds: true` in `ConfigureOptions`.
Never enable that escape hatch in a production app. Desktop/server release
builds continue to allow Test Store for development and CI.

## Release verification

Before claiming compatibility with a newly recorded upstream version:

1. Review the full upstream diff, including model and store code outside the
   networking directories.
2. Add or update wire fixtures and make the new tests fail against the old
   implementation.
3. Run `cargo test --workspace` and the TypeScript build.
4. Compile the Android and Apple native targets.
5. Exercise Test Store plus App Store and Play sandbox purchase, pending,
   restore, renewal, and cancellation flows on devices.

Passing mock tests establishes the local protocol contract. It does not by
itself establish current production-store compatibility. Until step 5 is run
for App Store and Play, the real-store bridges remain implementation- and
build-verified rather than device-E2E-verified.
