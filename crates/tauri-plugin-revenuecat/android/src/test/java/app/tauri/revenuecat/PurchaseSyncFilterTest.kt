package app.tauri.revenuecat

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class PurchaseSyncFilterTest {
    @Test
    fun `includes purchased transaction that is not acknowledged`() {
        assertTrue(
            shouldSyncPendingPurchase(
                state = OwnedPurchaseState.PURCHASED,
                isAcknowledged = false,
            ),
        )
    }

    @Test
    fun `excludes acknowledged purchase`() {
        assertFalse(
            shouldSyncPendingPurchase(
                state = OwnedPurchaseState.PURCHASED,
                isAcknowledged = true,
            ),
        )
    }

    @Test
    fun `excludes store pending transaction`() {
        assertFalse(
            shouldSyncPendingPurchase(
                state = OwnedPurchaseState.PENDING,
                isAcknowledged = false,
            ),
        )
    }

    @Test
    fun `excludes unspecified transaction`() {
        assertFalse(
            shouldSyncPendingPurchase(
                state = null,
                isAcknowledged = false,
            ),
        )
    }
}
