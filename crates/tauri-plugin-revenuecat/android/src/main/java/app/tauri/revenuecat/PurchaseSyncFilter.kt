package app.tauri.revenuecat

internal fun shouldSyncPendingPurchase(
    state: OwnedPurchaseState?,
    isAcknowledged: Boolean,
): Boolean = state == OwnedPurchaseState.PURCHASED && !isAcknowledged
