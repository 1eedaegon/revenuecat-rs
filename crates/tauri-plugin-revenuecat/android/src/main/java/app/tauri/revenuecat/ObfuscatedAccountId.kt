@file:OptIn(ExperimentalEncodingApi::class)

package app.tauri.revenuecat

import java.security.MessageDigest
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi

/** Matches purchases-android 11.1's SHA-256 plus Base64.NO_WRAP account identifier. */
internal fun String.toObfuscatedAccountId(): String = Base64.Default.encode(
    MessageDigest.getInstance("SHA-256").digest(toByteArray(Charsets.UTF_8)),
)

internal fun obfuscatedAccountIdForPurchase(
    appUserId: String?,
    productId: String,
    isSubscription: Boolean,
    ownedPurchases: List<OwnedPurchaseSnapshot>,
): String? {
    if (appUserId.isNullOrBlank()) return null
    val isAlreadyOwnedSubscription = isSubscription && ownedPurchases.any { purchase ->
        purchase.state == OwnedPurchaseState.PURCHASED && productId in purchase.productIds
    }
    if (isAlreadyOwnedSubscription) return null
    return appUserId.toObfuscatedAccountId()
}
