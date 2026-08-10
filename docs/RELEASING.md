# Releasing (maintainers)

CI (`.github/workflows/ci.yml`) runs fmt + clippy + tests + `cargo deny` on
every push. Publishing to crates.io is tag-driven
(`.github/workflows/release.yml`):

```sh
# once: add your crates.io token as a repo secret
gh secret set CARGO_REGISTRY_TOKEN

# release: bump [workspace.package] version, then
git tag v0.2.0 && git push origin v0.2.0
```

The release workflow verifies the tag matches `Cargo.toml`, re-runs the test
suite, and runs `cargo publish -p revenuecat-rs`.

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
