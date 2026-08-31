# Upstream SDK changes

## purchases-ios: 5.83.1 -> 5.87.1

- Release notes: https://github.com/RevenueCat/purchases-ios/releases/tag/5.87.1
- Full diff: https://github.com/RevenueCat/purchases-ios/compare/5.83.1...5.87.1
- No changes under protocol-relevant paths.

## purchases-android: 10.16.1 -> 10.19.1

- Release notes: https://github.com/RevenueCat/purchases-android/releases/tag/10.19.1
- Full diff: https://github.com/RevenueCat/purchases-android/compare/10.16.1...10.19.1
- Protocol-relevant files changed:
```
purchases/src/main/kotlin/com/revenuecat/purchases/common/Backend.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/Config.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/CustomVariableKeyValidator.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/Dispatcher.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/HTTPClient.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/JsonProvider.kt
purchases/src/main/kotlin/com/revenuecat/purchases/common/StorePlatform.kt
```

## purchases-js: 1.51.2 -> 1.55.0

- Release notes: https://github.com/RevenueCat/purchases-js/releases/tag/1.55.0
- Full diff: https://github.com/RevenueCat/purchases-js/compare/1.51.2...1.55.0
- Protocol-relevant files changed:
```
src/networking/backend.ts
src/networking/endpoints.ts
src/networking/http-client.ts
src/networking/responses/checkout-complete-response.ts
src/networking/responses/checkout-prepare-response.ts
src/networking/responses/checkout-start-response.ts
src/networking/responses/checkout-status-response.ts
src/networking/responses/paddle-checkout-settings.ts
src/networking/responses/subscription-change-response.ts
src/entities/attributes.ts
src/entities/branding.ts
src/entities/custom-paywall-impression-params.ts
src/entities/present-paywall-params.ts
src/entities/purchase-params.ts
src/entities/purchases-config.ts
src/entities/redemption-info.ts
```

