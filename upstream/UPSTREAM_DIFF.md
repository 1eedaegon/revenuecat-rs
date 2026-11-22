# Upstream SDK changes

## purchases-ios: 5.102.1 -> 5.103.2

- Release notes: https://github.com/RevenueCat/purchases-ios/releases/tag/5.103.2
- Full diff: https://github.com/RevenueCat/purchases-ios/compare/5.102.1...5.103.2
- Protocol-relevant files changed:
```
Sources/Ads/RewardVerification/Networking/GetRewardVerificationStatusOperation.swift
Sources/Ads/RewardVerification/Networking/RewardVerificationStatusResponse.swift
Sources/Ads/RewardVerification/Outcome.swift
Sources/Ads/RewardVerification/Poller.swift
Sources/Ads/RewardVerification/RewardVerificationPollStatus.swift
Sources/Checkpoints/DefaultCheckpointWorkflowResolver+Ad.swift
Sources/Checkpoints/ResolvedAdStep.swift
Sources/Error Handling/StoreKitError+Extensions.swift
Sources/FoundationExtensions/UIApplication+RCExtensions.swift
Sources/Identity/CustomerInfoManager.swift
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

## purchases-android: 11.1.0 -> 11.4.2

- Release notes: https://github.com/RevenueCat/purchases-android/releases/tag/11.4.2
- Full diff: https://github.com/RevenueCat/purchases-android/compare/11.1.0...11.4.2
- Protocol-relevant files changed:
```
purchases/src/main/kotlin/com/revenuecat/purchases/PackagePurchaseReference.kt
purchases/src/main/kotlin/com/revenuecat/purchases/PostReceiptHelper.kt
purchases/src/main/kotlin/com/revenuecat/purchases/PurchasesFactory.kt
purchases/src/main/kotlin/com/revenuecat/purchases/PurchasesOrchestrator.kt
purchases/src/main/kotlin/com/revenuecat/purchases/RewardVerificationPollStatus.kt
purchases/src/main/kotlin/com/revenuecat/purchases/ads/ConfiguredAdRewards.kt
purchases/src/main/kotlin/com/revenuecat/purchases/ads/ConfiguredAdRewardsProvider.kt
purchases/src/main/kotlin/com/revenuecat/purchases/amazon/AmazonBilling.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/Backend.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/BillingAbstract.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/Config.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/caching/DeviceCache.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/diagnostics/DiagnosticsEntryName.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/diagnostics/DiagnosticsSynchronizer.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/diagnostics/DiagnosticsTracker.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/localrules/LocalRulesDimensionProviders.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/networking/RewardVerificationResponse.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/networking/TokenAPI.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/networking/TokenOperations.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/offerings/OfferingsManager.kt
purchases/src/main/kotlin/com/revenuecat/purchases/google/BillingWrapper.kt
purchases/src/main/kotlin/com/revenuecat/purchases/google/usecase/BillingClientUseCase.kt
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
purchases/src/main/kotlin/com/revenuecat/purchases/simulatedstore/SimulatedStoreBillingWrapper.kt
purchases/src/main/kotlin/com/revenuecat/purchases/strings/BillingStrings.kt
purchases/src/main/kotlin/com/revenuecat/purchases/strings/NetworkStrings.kt
purchases/src/main/kotlin/com/revenuecat/purchases/utils/serializers/NullToEmptyListSerializer.kt
```

## purchases-js: 1.74.0 -> 1.78.0

- Release notes: https://github.com/RevenueCat/purchases-js/releases/tag/1.78.0
- Full diff: https://github.com/RevenueCat/purchases-js/compare/1.74.0...1.78.0
- Protocol-relevant files changed:
```
src/behavioural-events/event.ts
src/behavioural-events/events-tracker.ts
src/behavioural-events/flush-manager.ts
src/behavioural-events/paywall-event.ts
src/behavioural-events/sdk-event-context.ts
src/behavioural-events/sdk-event-helpers.ts
src/behavioural-events/sdk-events.ts
src/entities/customer-info.ts
src/entities/errors.ts
src/entities/external-purchase.ts
src/entities/offerings.ts
src/entities/virtual-currencies.ts
src/entities/virtual-currency.ts
src/external/external-purchases-service.ts
src/helpers/browser-globals.ts
src/helpers/constants.ts
src/helpers/decorators.ts
src/helpers/discount-breakdown-helper.ts
src/helpers/in-memory-cache.ts
src/helpers/offerings-parser.ts
src/helpers/paywall-discount-helpers.ts
src/helpers/paywall-offer-helpers.ts
src/helpers/paywall-package-info-helpers.ts
src/helpers/paywall-period-helpers.ts
src/helpers/paywall-price-helpers.ts
src/helpers/paywall-variables-helpers.ts
src/helpers/price-labels.ts
src/helpers/purchase-operation-helper.ts
src/helpers/simulated-store-post-receipt-helper.ts
src/helpers/simulated-store-purchase-helper.ts
src/helpers/uuid-helper.ts
src/main.ts
src/networking/backend.ts
src/networking/endpoints.ts
src/networking/http-client.ts
src/networking/responses/branding-response.ts
src/networking/responses/checkout-start-response.ts
src/networking/responses/checkout-status-response.ts
src/networking/responses/payrails-gateway-params.ts
src/networking/responses/subscriber-response.ts
```

