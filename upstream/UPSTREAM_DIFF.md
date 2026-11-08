# Upstream SDK changes

## purchases-ios: 5.102.1 -> 5.102.3

- Release notes: https://github.com/RevenueCat/purchases-ios/releases/tag/5.102.3
- Full diff: https://github.com/RevenueCat/purchases-ios/compare/5.102.1...5.102.3
- Protocol-relevant files changed:
```
Sources/FoundationExtensions/UIApplication+RCExtensions.swift
Sources/Info.plist
Sources/Misc/SystemInfo.swift
Sources/Paywalls/PaywallCacheWarming.swift
Sources/Paywalls/WebBundleURLBatcher.swift
Sources/Purchasing/PresentationContext.swift
Sources/Purchasing/Purchases/PurchaseParams.swift
Sources/Purchasing/Purchases/PurchasesOrchestrator.swift
Sources/Purchasing/SimulatedStore/SimulatedStorePurchaseHandler.swift
Sources/Purchasing/SimulatedStore/SimulatedStorePurchaseUI.swift
Sources/Purchasing/StoreKit2/StoreKit2ConfirmInOptions.swift
Sources/Purchasing/StoreKit2/StoreKit2ProductPurchaser.swift
Sources/Purchasing/WorkflowManager.swift
```

## purchases-android: 11.1.0 -> 11.2.0

- Release notes: https://github.com/RevenueCat/purchases-android/releases/tag/11.2.0
- Full diff: https://github.com/RevenueCat/purchases-android/compare/11.1.0...11.2.0
- Protocol-relevant files changed:
```
purchases/src/main/kotlin/com/revenuecat/purchases/PurchasesOrchestrator.kt
purchases/src/main/kotlin/com/revenuecat/purchases/RewardVerificationPollStatus.kt
purchases/src/main/kotlin/com/revenuecat/purchases/ads/ConfiguredAdRewards.kt
purchases/src/main/kotlin/com/revenuecat/purchases/ads/ConfiguredAdRewardsProvider.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/Backend.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/Config.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/caching/DeviceCache.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/diagnostics/DiagnosticsEntryName.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/diagnostics/DiagnosticsTracker.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/networking/RewardVerificationResponse.kt
purchases/src/main/kotlin/com/revenuecat/purchases/google/BillingWrapper.kt
purchases/src/main/kotlin/com/revenuecat/purchases/interfaces/GetAdRewardsCallback.kt
purchases/src/main/kotlin/com/revenuecat/purchases/strings/NetworkStrings.kt
```

