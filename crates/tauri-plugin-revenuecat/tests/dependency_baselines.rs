const CARGO_MANIFEST: &str = include_str!("../Cargo.toml");
const ANDROID_MANIFEST: &str = include_str!("../android/build.gradle.kts");
const DEMO_MANIFEST: &str = include_str!("../../../demo/tauri-app/src-tauri/Cargo.toml");
const CI_WORKFLOW: &str = include_str!("../../../.github/workflows/ci.yml");
const ROOT_README: &str = include_str!("../../../README.md");
const PLUGIN_README: &str = include_str!("../README.md");

#[test]
fn rust_manifest_uses_current_tauri_2_patch_floors() {
    assert!(
        CARGO_MANIFEST.contains(r#"tauri-plugin = { version = "2.7.2", features = ["build"] }"#)
    );
    assert_eq!(
        CARGO_MANIFEST
            .matches(r#"tauri = { version = "2.12.5""#)
            .count(),
        2
    );
}

#[test]
fn javascript_manifest_uses_current_tauri_api_and_compiler() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../package.json")).expect("package.json is valid JSON");

    assert_eq!(manifest["dependencies"]["@tauri-apps/api"], "^2.12.4");
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
