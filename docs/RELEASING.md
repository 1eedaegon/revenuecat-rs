# Releasing (maintainers)

Publishing to crates.io is tag-driven (`.github/workflows/release.yml`):
bump `[workspace.package] version`, the `revenuecat-rs` version under
`[workspace.dependencies]`, and `crates/tauri-plugin-revenuecat/package.json`
to the same value, run `cargo update -w`, then

```sh
git tag vX.Y.Z && git push origin vX.Y.Z
```

The release workflow verifies the tag matches `Cargo.toml`, re-runs the test
suite, checks the packaged crates, and publishes `revenuecat-rs`, then
`tauri-plugin-revenuecat` to crates.io and npm. The tag must be reachable from
`main`.

## Protocol-currency automations

- **Upstream watch** (`.github/workflows/upstream-watch.yml`): weekly cron
  compares the pinned versions in `upstream/versions.json` against the latest
  purchases-ios/android/js releases and opens an automated PR (via
  `peter-evans/create-pull-request`) with release notes, compare links, and
  the protocol-relevant files that changed. PRs opened with the default
  `GITHUB_TOKEN` don't trigger CI — set a `PAT` repo secret to get checks on
  those PRs, and enable "Allow GitHub Actions to create and approve pull
  requests" under Settings → Actions.
- **Dependabot** (`.github/dependabot.yml`): weekly cargo + actions updates,
  with `tauri*` grouped, so the demo stays on the newest Tauri.
- **Google Play Billing**: the upstream watcher covers the three RevenueCat
  repositories, not Google's Maven artifact. Check the official
  [Play Billing release notes](https://developer.android.com/google/play/billing/release-notes)
  and keep the Android Gradle dependency plus `docs/COMPATIBILITY.md` aligned.
