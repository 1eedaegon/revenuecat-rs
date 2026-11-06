# Plugin dependency refresh

## PRD

Keep `tauri-plugin-revenuecat` on the newest stable Tauri 2 toolchain without
changing its direct-port architecture. Update build-only dependencies, retain
the latest native store libraries, and give users copy-paste installation and
run commands in the READMEs.

Success means the Rust, TypeScript, Android, and iOS surfaces still build and
their existing tests pass on the declared Rust 1.90 floor.

## Architecture

The dependency refresh preserves the existing ownership boundary:

- Rust owns RevenueCat HTTP, identity, caching, and receipt orchestration.
- Tauri Rust commands own IPC validation, lifecycle, and error translation.
- Kotlin and Swift remain narrow StoreBilling bridges.
- The TypeScript package remains a typed wrapper over Tauri commands and
  events.

Tauri 3 prereleases are intentionally excluded. StoreKit remains provided by
the operating system, and Play Billing 9.1.0 remains the current stable Google
artifact.

## System design

| Layer | Stable baseline | Decision |
|---|---:|---|
| Tauri runtime | 2.12.5 | Raise the Rust manifest floor to the tested patch |
| Tauri plugin build API | 2.7.2 | Raise the build dependency floor |
| Tauri JavaScript API | 2.12.4 | Raise the npm dependency floor |
| TypeScript compiler | 7.0.2 | Use the current stable CLI compiler |
| Google Play Billing | 9.1.0 | Already current; keep the Java artifact |
| Swift tools | 5.5 | Keep the compatible official-plugin baseline |

Version-specific behavior belongs in the Tauri/Rust layer when possible so
that platform shims stay small and store callbacks continue to cross one typed
boundary.

## Technical plan

1. Add dependency-baseline regression checks before changing manifests.
2. Raise direct version floors and refresh the Cargo lockfile.
3. Run TypeScript 7 and fix compiler incompatibilities in the typed wrapper.
4. Fix any Tauri API incompatibility in Rust, not by adding native workarounds.
5. Update root and plugin READMEs with one-line commands and complete examples.
6. Run formatting, linting, unit/integration tests, dependency policy, and
   native target checks.

## Task list

- [x] Dependency baseline tests fail against the old manifest floors.
- [x] Rust and npm dependency floors match the stable versions above.
- [x] Compatible Rust transitive dependencies are refreshed.
- [x] TypeScript declaration output is current and tested.
- [x] README commands and examples match the public API.
- [x] Rust 1.90 local compatibility check passes.
- [x] Android and iOS validation remains enforced by pull-request CI.
