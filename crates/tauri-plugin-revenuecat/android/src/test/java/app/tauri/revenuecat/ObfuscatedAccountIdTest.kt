package app.tauri.revenuecat

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull

class ObfuscatedAccountIdTest {
    @Test
    fun `hashes app user id using RevenueCat SHA-256 Base64 semantics`() {
        val appUserId = "\$RCAnonymousID:1234567890"

        val obfuscatedAccountId = appUserId.toObfuscatedAccountId()

        assertEquals("xKv0AiHtlcwt6Do14WPQGUrMX7RkbkErmamccY5cGbQ=", obfuscatedAccountId)
        assertFalse(obfuscatedAccountId.contains(appUserId))
    }

    @Test
    fun `uses UTF-8 consistently for non-ASCII app user ids`() {
        assertEquals("CvqA2g9pMePwpvnq8IAqlKJpRyw61eUDW5/13DoBslU=", "사용자-42".toObfuscatedAccountId())
    }

    @Test
    fun `omits attribution when an older caller does not supply an app user id`() {
        assertNull(
            obfuscatedAccountIdForPurchase(
                appUserId = null,
                productId = "premium",
                isSubscription = false,
                ownedPurchases = emptyList(),
            ),
        )
    }

    @Test
    fun `sets attribution for new purchases and skips an already owned subscription`() {
        val completedSubscription = OwnedPurchaseSnapshot(
            purchaseToken = "owned-token",
            productIds = setOf("premium"),
            state = OwnedPurchaseState.PURCHASED,
        )

        assertEquals(
            "tMmiiTI7IaAcPpQPFQ65uMVCWH8av9jw4cwf/F5HVRQ=",
            obfuscatedAccountIdForPurchase(
                appUserId = "user@example.com",
                productId = "premium",
                isSubscription = false,
                ownedPurchases = listOf(completedSubscription),
            ),
        )
        assertEquals(
            "tMmiiTI7IaAcPpQPFQ65uMVCWH8av9jw4cwf/F5HVRQ=",
            obfuscatedAccountIdForPurchase(
                appUserId = "user@example.com",
                productId = "new-subscription",
                isSubscription = true,
                ownedPurchases = listOf(completedSubscription),
            ),
        )
        assertNull(
            obfuscatedAccountIdForPurchase(
                appUserId = "user@example.com",
                productId = "premium",
                isSubscription = true,
                ownedPurchases = listOf(completedSubscription),
            ),
        )
    }
}
