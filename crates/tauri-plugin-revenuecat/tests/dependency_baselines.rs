// Reads repository files outside this crate, so the manifest excludes this file
// from the published package.
const CARGO_MANIFEST: &str = include_str!("../Cargo.toml");
const WORKSPACE_MANIFEST: &str = include_str!("../../../Cargo.toml");
const ANDROID_MANIFEST: &str = include_str!("../android/build.gradle.kts");
const DEMO_MANIFEST: &str = include_str!("../../../demo/tauri-app/src-tauri/Cargo.toml");
const CI_WORKFLOW: &str = include_str!("../../../.github/workflows/ci.yml");
const ROOT_README: &str = include_str!("../../../README.md");
const PLUGIN_README: &str = include_str!("../README.md");
const IOS_PLUGIN: &str = include_str!("../ios/Sources/RevenueCatPlugin/RevenueCatPlugin.swift");

#[test]
fn rust_manifest_floors_are_the_oldest_tauri_patches_with_required_fixes() {
    // 2.7.1 lets docs.rs build plugins that ship native mobile projects.
    assert!(
        CARGO_MANIFEST.contains(r#"tauri-plugin = { version = "2.7.1", features = ["build"] }"#)
    );
    // 2.12.4 stops `run_mobile_plugin` aborting the app on a late native reply.
    assert_eq!(
        CARGO_MANIFEST
            .matches(r#"tauri = { version = "2.12.4""#)
            .count(),
        2
    );
}

#[test]
fn plugin_requires_the_sdk_release_it_is_built_against() {
    let release_version = WORKSPACE_MANIFEST
        .split_once("[workspace.package]\nversion = \"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(version, _)| version)
        .expect("workspace manifest declares the release version first");

    // The plugin calls SDK APIs added in the same release, so an older locked
    // SDK that merely satisfies "1" would not compile.
    assert!(WORKSPACE_MANIFEST.contains(&format!(
        r#"revenuecat-rs = {{ path = "crates/revenuecat", version = "{release_version}" }}"#
    )));
}

#[test]
fn published_package_excludes_tests_that_read_repository_files() {
    assert!(CARGO_MANIFEST.contains(r#"exclude = [".tauri", "tests/dependency_baselines.rs"]"#));
}

#[test]
fn ios_entry_point_stays_global_in_xcode_27_release_builds() {
    assert!(IOS_PLUGIN.contains("@_cdecl(\"init_plugin_revenuecat\")\npublic func initPlugin()"));
}

#[test]
fn javascript_manifest_uses_fixed_tauri_api_floor_and_current_compiler() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../package.json")).expect("package.json is valid JSON");

    // 2.12.3 keeps plugin listener channels delivering after a handler throws.
    assert_eq!(manifest["dependencies"]["@tauri-apps/api"], "^2.12.3");
    assert_eq!(manifest["devDependencies"]["typescript"], "^7.0.2");
}

#[test]
fn android_manifest_uses_current_base_billing_artifact() {
    assert!(ANDROID_MANIFEST.contains("com.android.billingclient:billing:9.1.0"));
    assert!(!ANDROID_MANIFEST.contains("implementation(\"com.android.billingclient:billing-ktx"));
}

#[test]
fn demo_and_ci_use_matching_tauri_2_tools() {
    assert!(DEMO_MANIFEST.contains(r#"tauri-build = { version = "2.7.4""#));
    assert_eq!(
        DEMO_MANIFEST
            .matches(r#"tauri = { version = "2.12.5""#)
            .count(),
        2
    );
    assert!(CI_WORKFLOW.contains("@tauri-apps/cli@2.12.4 android init"));
}

#[test]
fn readmes_provide_copy_paste_install_and_run_commands() {
    for command in [
        "npm install tauri-plugin-revenuecat && cargo add --manifest-path src-tauri/Cargo.toml tauri-plugin-revenuecat",
        "cargo add --manifest-path src-tauri/Cargo.toml tauri-plugin-revenuecat revenuecat-rs",
        "npm run tauri android dev",
        "npm run tauri ios dev",
    ] {
        assert!(ROOT_README.contains(command), "root README is missing {command}");
        assert!(
            PLUGIN_README.contains(command),
            "plugin README is missing {command}"
        );
    }
}
