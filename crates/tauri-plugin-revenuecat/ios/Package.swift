// swift-tools-version:5.5
import PackageDescription

let package = Package(
    name: "tauri-plugin-revenuecat",
    platforms: [
        .iOS(.v15),
        .macOS(.v12)
    ],
    products: [
        .library(
            name: "tauri-plugin-revenuecat",
            type: .static,
            targets: ["tauri-plugin-revenuecat"]
        )
    ],
    dependencies: [
        // Injected by the Tauri CLI into the app's build tree.
        .package(name: "Tauri", path: "../.tauri/tauri-api")
    ],
    targets: [
        .target(
            name: "tauri-plugin-revenuecat",
            dependencies: [
                .byName(name: "Tauri")
            ],
            path: "Sources/RevenueCatPlugin"
        )
    ]
)
