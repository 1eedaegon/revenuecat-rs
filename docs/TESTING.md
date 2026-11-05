# Test strategy

This project is a direct Rust/Kotlin/Swift/TypeScript port. A green Rust suite
therefore proves only part of the purchase contract: store callbacks, durable
recovery, IPC normalization, and identity transitions must also be tested at
their own boundaries.

## Required pull-request tests

All pull-request tests must be deterministic, use a mock backend or simulated
store, and avoid production purchases.

| Risk | Required assertion | Test layer |
|---|---|---|
| Lost or duplicated purchase | A receipt is durable before finish/acknowledge; finish failure keeps the recovery barrier; retry converges exactly once | Rust integration, Kotlin/Swift unit |
| Wrong-user attribution | Pending login/logout resumes to the exact target; a purchase cannot cross an unsettled identity transition | Rust integration |
| Corrupt local state | Malformed, unsupported, oversized, or internally inconsistent journals fail closed and do not look empty | Rust/Kotlin/Swift unit |
| Native callback ambiguity | Blank tokens and incomplete product/plan metadata reject the active purchase and trigger recovery | Kotlin unit, Rust IPC unit |
| Verification downgrade | Unknown entitlement verification modes are rejected rather than silently becoming informational | Rust/Tauri unit |
| Public API drift | TypeScript names and payloads match the Rust wire shape, including `Offerings.current` normalization | TypeScript type/contract unit |
| Lifecycle race | Partial listener installation is cleaned up; focus/resume/native events coalesce and failed sync retries | TypeScript state-machine unit |
| Secret leakage | Debug output and errors redact receipt tokens, signed transactions, and App User IDs | Rust unit |

### Rust core and Tauri IPC

- Configuration limits, API-key/store selection, verification modes, and
  release-build Test Store guards.
- Backend paths, headers, receipt bodies, error-code parsing, 429 retry, ETag
  replay, and signature verification against `revenuecat-mock`.
- Cache isolation across identities and stale offering/package rejection.
- Purchase success, cancellation, pending, backend failure, finish failure,
  partial queue success, recovery barriers, restore, and exact-once
  convergence.
- Durable identity prepare/commit/resume, owner-only journal permissions, and
  corrupt/oversized journal rejection.
- Tauri command names, camel-case wire payloads, idempotent configure, rejected
  conflicting configure, and inaccessible internal native commands.

### TypeScript guest API

- Compile-time public contract tests for exported types.
- Exact `invoke` command names, argument shapes, listener payloads, and error
  propagation for every public wrapper.
- Test Store configure performs no native listener registration or native
  synchronization.
- App Store and Play configure perform the correct initial sync.
- Recovery blocks purchase/login/logout until pending receipts settle.
- Native listener and lifecycle listener partial failures clean up and allow a
  later configure retry.
- `Offerings.current` is derived from `current_offering_id` while the raw wire
  field remains available for 1.x compatibility.

The JavaScript suite enforces at least 80% line, branch, and function coverage.

### Android bridge

- Subscription option selection, base-plan/offer propagation, and stale-plan
  rejection.
- Obfuscated account ID stability without exposing the raw App User ID.
- Active versus recovered purchase routing, duplicate token handling, blank
  token rejection, and purchase serialization.
- Journal schema/version/size validation, strict replacement, and persistence
  failure propagation.
- Journal deletion happens only after successful consume/acknowledge.
- Sandbox/release guard and native error mapping.

### Apple bridge

- StoreKit product/transaction mapping and UUID-only `appAccountToken`.
- Atomic claim of pending purchase context: concurrent promotions have one
  winner, and rollback cannot overwrite a newer attempt.
- Journal schema/version/size validation, dictionary-key consistency, and
  rejection of the same transaction ID with a different JWS, product, or
  receipt attribution context.
- Receipt durability before `finish()`, plus finish-failure recovery.
- Pending, cancellation, unverified transaction, and listener lifecycle paths.

## Local and CI commands

Run the same gates used by CI:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo deny check advisories licenses bans sources

cd crates/tauri-plugin-revenuecat
npm install --no-audit --no-fund --no-package-lock
npm run build
npm test
```

The Android bridge is tested from the generated Tauri Android host:

```sh
cd demo/tauri-app/src-tauri
npx --yes @tauri-apps/cli@2.12.2 android init --ci --skip-targets-install
cd gen/android
./gradlew :tauri-plugin-revenuecat:testDebugUnitTest \
  :tauri-plugin-revenuecat:lintDebug
```

The Apple bridge requires an available iOS simulator:

```sh
cd crates/tauri-plugin-revenuecat/ios
simulator_id=$(xcrun simctl list devices available -j | \
  jq -r 'first(.devices[][] | select(.name | startswith("iPhone")) | .udid) // empty')
test -n "$simulator_id"
xcodebuild -scheme tauri-plugin-revenuecat \
  -destination "platform=iOS Simulator,id=$simulator_id" test
```

For a measurable Rust coverage gate, install `cargo-llvm-cov` and run:

```sh
cargo llvm-cov --workspace --all-targets --fail-under-lines 80
```

## Nightly and release-candidate tests

These tests depend on credentials, packaged applications, simulators, or
external services. They should not make ordinary pull requests flaky.

### RevenueCat Test Store

- Configure, offerings, purchase, customer-info refresh, restore, login, and
  logout on desktop and packaged iOS/Android Tauri apps.
- Kill the app after receipt creation and before completion, then verify launch
  recovery without a second entitlement grant.
- Verify mobile release builds reject `test_` keys unless the explicit
  dangerous test-only opt-in is set.
- Verify logs and artifacts contain no API key, receipt, signed transaction, or
  App User ID.

### StoreKit testing and App Store sandbox

- Consumable, non-consumable, and auto-renewing subscription purchases.
- Introductory/promotional offer selection and upgrade/downgrade behavior.
- Ask-to-Buy/pending, cancellation, unverified transaction, interrupted
  network, process death, reinstall, restore, renewal, expiration, billing
  retry, refund, and revocation.
- Finish failure followed by foreground/launch recovery.

### Play internal test track

- In-app product and subscription purchase for each base plan/offer type.
- Pending cash purchase, cancellation, already-owned item, network loss,
  process death, reinstall, restore/full active scan, renewal, grace period,
  account hold, refund, and revocation.
- Acknowledge/consume failure followed by recovery, duplicate callback, blank or
  stale callback data, and a purchase made outside the current process.

### Packaged Tauri application

- RevenueCat permissions are granted only to the intended purchase window.
- A strict CSP is active and untrusted content cannot invoke payment commands.
- Foreground/resume and native transaction events reach the guest state machine
  on real devices.

## Release evidence

For each real-store run, record the commit, app build, OS/device, store tooling
version, scenario, anonymized transaction result, backend entitlement result,
and recovery outcome. Never attach raw API keys, purchase tokens, JWS values,
or App User IDs.

Mock and Test Store success establishes protocol and application integration.
Only successful App Store sandbox and Play test-track runs establish current
native-store compatibility for a release candidate.
