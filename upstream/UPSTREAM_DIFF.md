# Upstream SDK changes

## purchases-ios: 5.83.1 -> 5.83.2

- Release notes: https://github.com/RevenueCat/purchases-ios/releases/tag/5.83.2
- Full diff: https://github.com/RevenueCat/purchases-ios/compare/5.83.1...5.83.2
- Protocol-relevant files changed:
```
Sources/Networking/AudiencesConfigProvider.swift
Sources/Networking/CheckpointRules.swift
Sources/Networking/CheckpointsConfigProvider.swift
Sources/Networking/RemoteConfigManager.swift
Sources/Networking/RemoteConfigTopic.swift
Sources/Networking/UiConfigProvider.swift
Sources/Networking/WorkflowsConfigProvider.swift
```

## purchases-android: 10.16.1 -> 10.16.2

- Release notes: https://github.com/RevenueCat/purchases-android/releases/tag/10.16.2
- Full diff: https://github.com/RevenueCat/purchases-android/compare/10.16.1...10.16.2
- Protocol-relevant files changed:
```
purchases/src/main/kotlin/com/revenuecat/purchases/common/Config.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/audiences/AudiencesConfigProvider.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/checkpoints/CheckpointResponse.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/checkpoints/CheckpointRulesResolution.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/checkpoints/CheckpointsConfigProvider.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/localrules/LocalRule.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/localrules/LocalRulesEvaluator.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/localrules/RulesDimensionProvider.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/localrules/RulesDimensionResolver.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/localrules/RulesEngineLoggerBridge.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/offlineentitlements/ProductEntitlementMapping.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/remoteconfig/RemoteConfigManager.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/remoteconfig/RemoteConfigTopic.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/uiconfig/UiConfigProvider.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/utils.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/workflows/WorkflowAssetPrewarmer.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/workflows/WorkflowManager.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/workflows/WorkflowsConfigProvider.kt
```

## purchases-js: 1.51.2 -> 1.52.3

- Release notes: https://github.com/RevenueCat/purchases-js/releases/tag/1.52.3
- Full diff: https://github.com/RevenueCat/purchases-js/compare/1.51.2...1.52.3
- Protocol-relevant files changed:
```
src/networking/backend.ts
src/networking/endpoints.ts
src/networking/http-client.ts
src/networking/responses/checkout-complete-response.ts
src/networking/responses/checkout-prepare-response.ts
src/networking/responses/checkout-start-response.ts
src/networking/responses/checkout-status-response.ts
src/networking/responses/subscription-change-response.ts
src/entities/present-paywall-params.ts
src/entities/redemption-info.ts
```

