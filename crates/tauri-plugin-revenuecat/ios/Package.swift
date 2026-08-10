// swift-tools-version:5.5
import PackageDescription

let package = Package(
    name: "RevenueCatPlugin",
    platforms: [
        .iOS(.v15),
        .macOS(.v12)
    ],
    products: [
        .library(
            name: "RevenueCatPlugin",
            type: .static,
            targets: ["RevenueCatPlugin"]
        )
    ],
    dependencies: [
        // Injected by the Tauri CLI into the app's build tree.
        .package(name: "Tauri", path: "../.tauri/tauri-api")
    ],
    targets: [
        .target(
            name: "RevenueCatPlugin",
            dependencies: [
                .byName(name: "Tauri")
            ],
            path: ".",
            sources: ["RevenueCatPlugin.swift"]
        )
    ]
)
