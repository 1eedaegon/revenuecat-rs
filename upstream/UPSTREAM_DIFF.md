# Upstream SDK changes

## purchases-ios: 5.102.1 -> 5.103.0

- Release notes: https://github.com/RevenueCat/purchases-ios/releases/tag/5.103.0
- Full diff: https://github.com/RevenueCat/purchases-ios/compare/5.102.1...5.103.0
- Protocol-relevant files changed:
```
Sources/Ads/RewardVerification/Networking/GetRewardVerificationStatusOperation.swift
Sources/Ads/RewardVerification/Networking/RewardVerificationStatusResponse.swift
Sources/Ads/RewardVerification/Outcome.swift
Sources/Ads/RewardVerification/Poller.swift
Sources/Ads/RewardVerification/RewardVerificationPollStatus.swift
Sources/Checkpoints/DefaultCheckpointWorkflowResolver+Ad.swift
Sources/Checkpoints/ResolvedAdStep.swift
Sources/FoundationExtensions/UIApplication+RCExtensions.swift
Sources/Info.plist
Sources/Logging/Strings/AdsStrings.swift
Sources/Logging/Strings/BackendErrorStrings.swift
Sources/Misc/SystemInfo.swift
Sources/Paywalls/PaywallCacheWarming.swift
Sources/Paywalls/WebBundleURLBatcher.swift
Sources/Purchasing/PresentationContext.swift
Sources/Purchasing/Purchases/PurchaseParams.swift
Sources/Purchasing/Purchases/Purchases.swift
Sources/Purchasing/Purchases/PurchasesOrchestrator.swift
Sources/Purchasing/Purchases/PurchasesType.swift
Sources/Purchasing/SimulatedStore/SimulatedStorePurchaseHandler.swift
Sources/Purchasing/SimulatedStore/SimulatedStorePurchaseUI.swift
Sources/Purchasing/StoreKit2/StoreKit2ConfirmInOptions.swift
Sources/Purchasing/StoreKit2/StoreKit2ProductPurchaser.swift
Sources/Purchasing/WorkflowManager.swift
```

## purchases-android: 11.1.0 -> 11.3.0

- Release notes: https://github.com/RevenueCat/purchases-android/releases/tag/11.3.0
- Full diff: https://github.com/RevenueCat/purchases-android/compare/11.1.0...11.3.0
- Protocol-relevant files changed:
```
purchases/src/main/kotlin/com/revenuecat/purchases/PackagePurchaseReference.kt
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
purchases/src/main/kotlin/com/revenuecat/purchases/common/networking/TokenAPI.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/networking/TokenOperations.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/offerings/OfferingsManager.kt
purchases/src/main/kotlin/com/revenuecat/purchases/google/BillingWrapper.kt
purchases/src/main/kotlin/com/revenuecat/purchases/identity/IdentityManager.kt
purchases/src/main/kotlin/com/revenuecat/purchases/interfaces/GetAdRewardsCallback.kt
purchases/src/main/kotlin/com/revenuecat/purchases/interfaces/GetPackagesForPurchaseCallback.kt
purchases/src/main/kotlin/com/revenuecat/purchases/paywalls/components/ButtonComponent.kt
purchases/src/main/kotlin/com/revenuecat/purchases/paywalls/components/CarouselComponent.kt
purchases/src/main/kotlin/com/revenuecat/purchases/paywalls/components/IconComponent.kt
purchases/src/main/kotlin/com/revenuecat/purchases/paywalls/components/ImageComponent.kt
purchases/src/main/kotlin/com/revenuecat/purchases/paywalls/components/PackageComponent.kt
purchases/src/main/kotlin/com/revenuecat/purchases/paywalls/components/StackComponent.kt
purchases/src/main/kotlin/com/revenuecat/purchases/paywalls/components/TabsComponent.kt
purchases/src/main/kotlin/com/revenuecat/purchases/paywalls/components/TextComponent.kt
purchases/src/main/kotlin/com/revenuecat/purchases/paywalls/components/TimelineComponent.kt
purchases/src/main/kotlin/com/revenuecat/purchases/paywalls/components/WebViewComponent.kt
purchases/src/main/kotlin/com/revenuecat/purchases/strings/BillingStrings.kt
purchases/src/main/kotlin/com/revenuecat/purchases/strings/NetworkStrings.kt
purchases/src/main/kotlin/com/revenuecat/purchases/utils/serializers/NullToEmptyListSerializer.kt
```

