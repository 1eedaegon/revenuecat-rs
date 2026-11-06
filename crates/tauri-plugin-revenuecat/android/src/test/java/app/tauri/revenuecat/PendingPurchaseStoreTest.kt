package app.tauri.revenuecat

import android.content.SharedPreferences
import com.android.billingclient.api.BillingClient
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class PendingPurchaseStoreTest {
    @Test
    fun `pending update releases callback and blocks only the correlated product`() {
        val store = PendingPurchaseStore<String, String>()
        assertTrue(store.register("monthly", "invoke"))
        store.updateContext("monthly", "monthly:trial")

        val pendingCallback = store.defer(
            productIds = listOf("monthly"),
            purchaseToken = "pending-token",
        )
        assertEquals(
            PendingPurchase("invoke", "monthly:trial"),
            pendingCallback,
        )
        assertTrue(store.hasDeferredPurchase())
        assertTrue(store.hasCorrelatedPurchase("pending-token"))
        assertFalse(store.register("monthly", "new-invoke"))
        assertTrue(store.register("annual", "other-invoke"))
    }

    @Test
    fun `completed query returns retained context and clears deferred marker`() {
        val store = deferredStore()

        val contexts = store.reconcileOwnedPurchases(
            listOf(
                OwnedPurchaseSnapshot(
                    purchaseToken = "pending-token",
                    productIds = setOf("monthly"),
                    state = OwnedPurchaseState.PURCHASED,
                ),
            ),
        )

        assertEquals(mapOf("pending-token" to "monthly:trial"), contexts)
        assertFalse(store.hasDeferredPurchase())
        assertTrue(store.register("monthly", "next-invoke"))
    }

    @Test
    fun `pending query keeps deferred marker and does not expose context`() {
        val store = deferredStore()

        val contexts = store.reconcileOwnedPurchases(
            listOf(
                OwnedPurchaseSnapshot(
                    purchaseToken = "pending-token",
                    productIds = setOf("monthly"),
                    state = OwnedPurchaseState.PENDING,
                ),
            ),
        )

        assertTrue(contexts.isEmpty())
        assertTrue(store.hasDeferredPurchase())
        assertFalse(store.register("monthly", "next-invoke"))
    }

    @Test
    fun `query with no matching owned purchase clears deferred marker`() {
        val store = deferredStore()

        assertTrue(store.reconcileOwnedPurchases(emptyList()).isEmpty())

        assertFalse(store.hasDeferredPurchase())
        assertTrue(store.register("monthly", "next-invoke"))
    }

    @Test
    fun `mismatched product ids never inherit a subscription option`() {
        val store = deferredStore()

        val contexts = store.reconcileOwnedPurchases(
            listOf(
                OwnedPurchaseSnapshot(
                    purchaseToken = "pending-token",
                    productIds = setOf("annual"),
                    state = OwnedPurchaseState.PURCHASED,
                ),
            ),
        )

        assertTrue(contexts.containsKey("pending-token"))
        assertNull(contexts["pending-token"])
        assertFalse(store.hasDeferredPurchase())
    }

    @Test
    fun `completed update for deferred token cannot take an active callback`() {
        val store = deferredStore()
        assertTrue(store.register("annual", "new-invoke"))

        val route = store.routePurchased(
            purchaseToken = "pending-token",
            productIds = listOf("monthly"),
            purchaseTimeMillis = 100L,
        )

        assertEquals(PurchaseUpdateKind.CORRELATED, route.kind)
        assertNull(route.activePurchase)
        assertEquals(
            listOf(PendingPurchase("new-invoke", null)),
            store.takeForProducts(listOf("annual")),
        )
    }

    @Test
    fun `active purchase context survives queries until backend finish`() {
        val store = PendingPurchaseStore<String, String>()
        assertTrue(store.register("monthly", "invoke"))
        store.updateContext("monthly", "monthly:trial")
        store.markLaunched("monthly", 100L)

        val route = store.routePurchased(
            purchaseToken = "active-token",
            productIds = listOf("monthly"),
            purchaseTimeMillis = 100L,
        )
        val completed = OwnedPurchaseSnapshot(
            purchaseToken = "active-token",
            productIds = setOf("monthly"),
            state = OwnedPurchaseState.PURCHASED,
        )

        assertEquals(PurchaseUpdateKind.ACTIVE, route.kind)
        assertEquals(
            mapOf("active-token" to "monthly:trial"),
            store.reconcileOwnedPurchases(listOf(completed)),
        )
        assertEquals(
            mapOf("active-token" to "monthly:trial"),
            store.reconcileOwnedPurchases(listOf(completed)),
        )
        store.removeCorrelationBeforeStoreFinish("active-token")
        assertTrue(store.reconcileOwnedPurchases(listOf(completed)).isEmpty())
    }

    @Test
    fun `active purchase context survives a transient empty ownership query`() {
        val store = PendingPurchaseStore<String, String>()
        assertTrue(store.register("monthly", "invoke"))
        store.updateContext("monthly", "monthly:trial")
        store.markLaunched("monthly", 100L)
        store.routePurchased("active-token", listOf("monthly"), 100L)
        val completed = OwnedPurchaseSnapshot(
            purchaseToken = "active-token",
            productIds = setOf("monthly"),
            state = OwnedPurchaseState.PURCHASED,
        )

        assertTrue(store.reconcileOwnedPurchases(emptyList()).isEmpty())
        assertEquals(
            mapOf("active-token" to "monthly:trial"),
            store.reconcileOwnedPurchases(listOf(completed)),
        )
    }

    @Test
    fun `deferred purchase context survives a failed synchronization retry`() {
        val store = deferredStore()
        val completed = OwnedPurchaseSnapshot(
            purchaseToken = "pending-token",
            productIds = setOf("monthly"),
            state = OwnedPurchaseState.PURCHASED,
        )

        assertEquals(
            mapOf("pending-token" to "monthly:trial"),
            store.reconcileOwnedPurchases(listOf(completed)),
        )
        assertEquals(
            mapOf("pending-token" to "monthly:trial"),
            store.reconcileOwnedPurchases(listOf(completed)),
        )
    }

    @Test
    fun `selected option survives process death pending completion and retry until finish`() {
        val metadataStore = FakeLocalTransactionMetadataStore<String>()
        val firstProcess = PendingPurchaseStore<String, String>(metadataStore)
        assertTrue(firstProcess.register("monthly", "invoke"))
        firstProcess.updateContext("monthly", "monthly:trial")
        firstProcess.defer(listOf("monthly"), "pending-token")

        val pendingProcess = PendingPurchaseStore<String, String>(metadataStore)
        assertTrue(
            pendingProcess.reconcileOwnedPurchases(
                listOf(purchase("pending-token", "monthly", OwnedPurchaseState.PENDING)),
            ).isEmpty(),
        )

        val completedProcess = PendingPurchaseStore<String, String>(metadataStore)
        val completed = purchase("pending-token", "monthly", OwnedPurchaseState.PURCHASED)
        assertEquals(
            mapOf("pending-token" to "monthly:trial"),
            completedProcess.reconcileOwnedPurchases(listOf(completed)),
        )

        val retryProcess = PendingPurchaseStore<String, String>(metadataStore)
        assertEquals(
            mapOf("pending-token" to "monthly:trial"),
            retryProcess.reconcileOwnedPurchases(listOf(completed)),
        )

        retryProcess.removeCorrelationBeforeStoreFinish("pending-token")
        assertTrue(
            PendingPurchaseStore<String, String>(metadataStore)
                .reconcileOwnedPurchases(listOf(completed))
                .isEmpty(),
        )
    }

    @Test
    fun `persisted context is never attributed to mismatched or external token`() {
        val metadataStore = FakeLocalTransactionMetadataStore<String>()
        val original = PendingPurchaseStore<String, String>(metadataStore)
        assertTrue(original.register("monthly", "invoke"))
        original.updateContext("monthly", "monthly:trial")
        original.defer(listOf("monthly"), "pending-token")

        val restored = PendingPurchaseStore<String, String>(metadataStore)
        val contexts = restored.reconcileOwnedPurchases(
            listOf(
                purchase("pending-token", "annual", OwnedPurchaseState.PURCHASED),
                purchase("external-token", "monthly", OwnedPurchaseState.PURCHASED),
            ),
        )

        assertTrue(contexts.containsKey("pending-token"))
        assertNull(contexts["pending-token"])
        assertFalse(contexts.containsKey("external-token"))
        assertNull(metadataStore.metadata.single().context)

        val restarted = PendingPurchaseStore<String, String>(metadataStore)
        assertNull(
            restarted.reconcileOwnedPurchases(
                listOf(purchase("pending-token", "monthly", OwnedPurchaseState.PURCHASED)),
            )["pending-token"],
        )
    }

    @Test
    fun `ownership queries do not delete durable metadata before finish`() {
        val metadataStore = FakeLocalTransactionMetadataStore<String>()
        val store = PendingPurchaseStore<String, String>(metadataStore)
        assertTrue(store.register("monthly", "invoke"))
        store.updateContext("monthly", "monthly:trial")
        store.defer(listOf("monthly"), "pending-token")

        assertTrue(store.reconcileOwnedPurchases(emptyList()).isEmpty())
        assertEquals(setOf("pending-token"), metadataStore.metadata.map { it.purchaseToken }.toSet())

        store.removeCorrelationBeforeStoreFinish("pending-token")
        assertTrue(metadataStore.metadata.isEmpty())
    }

    @Test
    fun `purchase callback observes metadata commit failure`() {
        val metadataStore = FakeLocalTransactionMetadataStore<String>().apply {
            replaceSucceeds = false
        }
        val pendingStore = PendingPurchaseStore<String, String>(metadataStore)
        assertTrue(pendingStore.register("monthly", "pending-invoke"))
        pendingStore.updateContext("monthly", "monthly:trial")

        val pending = pendingStore.defer(listOf("monthly"), "pending-token")

        assertFalse(pending?.isMetadataDurable ?: true)

        val purchasedStore = PendingPurchaseStore<String, String>(metadataStore)
        assertTrue(purchasedStore.register("annual", "purchase-invoke"))
        purchasedStore.updateContext("annual", "annual:intro")

        val purchased = purchasedStore.routePurchased("purchase-token", listOf("annual"), 1L)

        assertEquals(PurchaseUpdateKind.ACTIVE, purchased.kind)
        assertFalse(purchased.isMetadataDurable)
        assertFalse(purchased.activePurchase?.isMetadataDurable ?: true)
    }

    @Test
    fun `pre-finish deletion failure preserves metadata and allows retry`() {
        val metadataStore = FakeLocalTransactionMetadataStore<String>()
        val store = PendingPurchaseStore<String, String>(metadataStore)
        assertTrue(store.register("monthly", "invoke"))
        store.updateContext("monthly", "monthly:trial")
        store.routePurchased("purchase-token", listOf("monthly"), 1L)
        metadataStore.replaceSucceeds = false

        assertFalse(store.removeCorrelationBeforeStoreFinish("purchase-token"))
        assertTrue(store.hasCorrelatedPurchase("purchase-token"))
        assertEquals("monthly:trial", metadataStore.metadata.single().context)

        metadataStore.replaceSucceeds = true
        assertTrue(store.removeCorrelationBeforeStoreFinish("purchase-token"))
        assertFalse(store.hasCorrelatedPurchase("purchase-token"))
        assertTrue(metadataStore.metadata.isEmpty())
    }

    @Test
    fun `finish retry treats item not owned as idempotent success`() {
        assertTrue(isIdempotentFinishSuccess(BillingClient.BillingResponseCode.OK))
        assertTrue(isIdempotentFinishSuccess(BillingClient.BillingResponseCode.ITEM_NOT_OWNED))
        assertFalse(isIdempotentFinishSuccess(BillingClient.BillingResponseCode.ERROR))
        assertFalse(shouldCallPlayAcknowledge(isOwned = false, isAcknowledged = false))
        assertFalse(shouldCallPlayAcknowledge(isOwned = true, isAcknowledged = true))
        assertTrue(shouldCallPlayAcknowledge(isOwned = true, isAcknowledged = false))
    }

    @Test
    fun `metadata store exception fails closed`() {
        val metadataStore = FakeLocalTransactionMetadataStore<String>().apply {
            replaceError = IllegalStateException("disk unavailable")
        }
        val store = PendingPurchaseStore<String, String>(metadataStore)
        assertTrue(store.register("monthly", "invoke"))
        store.updateContext("monthly", "monthly:trial")

        val routed = store.routePurchased("purchase-token", listOf("monthly"), 1L)

        assertFalse(routed.isMetadataDurable)
    }

    @Test
    fun `preflight and reconciliation surface tombstone persistence failure`() {
        val metadataStore = FakeLocalTransactionMetadataStore<String>()
        val store = PendingPurchaseStore<String, String>(metadataStore)
        assertTrue(store.register("monthly", "invoke"))
        store.updateContext("monthly", "monthly:trial")
        store.routePurchased("purchase-token", listOf("monthly"), 1L)
        metadataStore.replaceSucceeds = false
        val mismatched = purchase("purchase-token", "annual", OwnedPurchaseState.PURCHASED)

        assertEquals(
            PurchasePreparation.METADATA_PERSISTENCE_FAILED,
            store.prepareForPurchase("annual", listOf(mismatched)),
        )
        store.reconcileOwnedPurchases(listOf(mismatched))
        assertTrue(store.hasMetadataPersistenceFailure())
    }

    @Test
    fun `full native receipt context survives transaction retry`() {
        val receiptContext = NativeReceiptContext(
            presentedOfferingIdentifier = "default",
            presentedPlacementIdentifier = "onboarding",
            appliedTargetingRule = ReceiptTargetingRule("rule-1", 7L),
            priceMicros = 1_990_000L,
            currency = "KRW",
            normalDuration = "P1M",
            sdkOriginated = true,
        )
        val purchaseContext = NativePurchaseContext(
            productId = "pro",
            basePlanId = "monthly",
            offerId = "trial",
            receiptContext = receiptContext,
        )
        val metadataStore = FakeLocalTransactionMetadataStore<NativePurchaseContext>()
        val firstProcess = PendingPurchaseStore<String, NativePurchaseContext>(metadataStore)
        assertTrue(firstProcess.register("pro", "invoke"))
        firstProcess.updateContext("pro", purchaseContext)
        firstProcess.routePurchased("purchase-token", listOf("pro"), 1L)

        val retry = PendingPurchaseStore<String, NativePurchaseContext>(metadataStore)
        val contexts = retry.reconcileOwnedPurchases(
            listOf(purchase("purchase-token", "pro", OwnedPurchaseState.PURCHASED)),
        )

        assertEquals(purchaseContext, contexts["purchase-token"])
        assertEquals("default", contexts["purchase-token"]?.receiptContext?.presentedOfferingIdentifier)
        assertEquals("rule-1", contexts["purchase-token"]?.receiptContext?.appliedTargetingRule?.ruleId)
    }

    @Test
    fun `native receipt context rejects negative price at IPC boundary`() {
        val negativePrice = NativeReceiptContextArgs().apply {
            priceMicros = -1L
            currency = "KRW"
        }
        val zeroPrice = NativeReceiptContextArgs().apply {
            priceMicros = 0L
            currency = "KRW"
        }

        assertNull(negativePrice.toValidatedReceiptContext())
        assertEquals(0L, zeroPrice.toValidatedReceiptContext()?.priceMicros)
    }

    @Test
    fun `shared preferences journal rejects negative receipt price`() {
        val store = SharedPreferencesLocalTransactionMetadataStore(FakeSharedPreferences())
        val receiptContext = NativeReceiptContext(
            presentedOfferingIdentifier = null,
            presentedPlacementIdentifier = null,
            appliedTargetingRule = null,
            priceMicros = -1L,
            currency = "KRW",
            normalDuration = null,
            sdkOriginated = true,
        )
        val metadata = LocalTransactionMetadata(
            purchaseToken = "purchase-token",
            productIds = setOf("pro"),
            context = NativePurchaseContext("pro", "monthly", null, receiptContext),
        )

        assertFalse(store.replace(listOf(metadata)))
        assertTrue(store.load().isEmpty())
    }

    @Test
    fun `shared preferences journal round trips receipt context up to capacity`() {
        val preferences = FakeSharedPreferences()
        val store = SharedPreferencesLocalTransactionMetadataStore(preferences)
        val receiptContext = NativeReceiptContext(
            presentedOfferingIdentifier = "default",
            presentedPlacementIdentifier = "onboarding",
            appliedTargetingRule = ReceiptTargetingRule("rule-1", 7L),
            priceMicros = 1_990_000L,
            currency = "KRW",
            normalDuration = "P1M",
            sdkOriginated = true,
        )
        val metadata = (1..32).map { index ->
            LocalTransactionMetadata(
                purchaseToken = "token-$index",
                productIds = setOf("pro-$index"),
                context = NativePurchaseContext(
                    productId = "pro-$index",
                    basePlanId = "monthly",
                    offerId = "trial",
                    receiptContext = receiptContext,
                ),
            )
        }

        assertTrue(store.replace(metadata))
        val restored = SharedPreferencesLocalTransactionMetadataStore(preferences).load()

        assertEquals(32, restored.size)
        assertEquals(metadata.first(), restored.first())
        assertEquals(receiptContext, restored.first().context?.receiptContext)
    }

    @Test
    fun `shared preferences journal rejects mismatched product context`() {
        val preferences = FakeSharedPreferences()
        val store = SharedPreferencesLocalTransactionMetadataStore(preferences)
        val mismatched = LocalTransactionMetadata(
            purchaseToken = "token",
            productIds = setOf("monthly"),
            context = NativePurchaseContext(
                productId = "annual",
                basePlanId = "annual-base",
                offerId = null,
                receiptContext = null,
            ),
        )

        assertFalse(store.replace(listOf(mismatched)))
        assertTrue(store.load().isEmpty())
    }

    @Test
    fun `shared preferences commit failure is returned to caller`() {
        val preferences = FakeSharedPreferences().apply { commitSucceeds = false }
        val store = SharedPreferencesLocalTransactionMetadataStore(preferences)
        val metadata = LocalTransactionMetadata(
            purchaseToken = "token",
            productIds = setOf("monthly"),
            context = NativePurchaseContext("monthly", "base", null, null),
        )

        assertFalse(store.replace(listOf(metadata)))
        assertTrue(store.load().isEmpty())
    }

    @Test
    fun `restored durable metadata is bounded`() {
        val metadataStore = FakeLocalTransactionMetadataStore(
            initial = (1..40).map { index ->
                LocalTransactionMetadata(
                    purchaseToken = "token-$index",
                    productIds = setOf("product-$index"),
                    context = "context-$index",
                )
            },
        )
        val store = PendingPurchaseStore<String, String>(metadataStore)
        val owned = (1..40).map { index ->
            purchase("token-$index", "product-$index", OwnedPurchaseState.PURCHASED)
        }

        assertEquals(32, store.reconcileOwnedPurchases(owned).size)
    }

    @Test
    fun `preflight moves completed marker without losing context or blocking its product`() {
        val store = deferredStore()
        val completed = OwnedPurchaseSnapshot(
            purchaseToken = "pending-token",
            productIds = setOf("monthly"),
            state = OwnedPurchaseState.PURCHASED,
        )

        assertEquals(
            PurchasePreparation.READY,
            store.prepareForPurchase("monthly", listOf(completed)),
        )

        assertFalse(store.hasDeferredPurchase())
        assertTrue(store.hasCorrelatedPurchase("pending-token"))
        assertTrue(store.register("monthly", "new-invoke"))
        assertEquals(
            mapOf("pending-token" to "monthly:trial"),
            store.reconcileOwnedPurchases(listOf(completed)),
        )
        assertTrue(store.hasCorrelatedPurchase("pending-token"))
        store.removeCorrelationBeforeStoreFinish("pending-token")
        assertFalse(store.hasCorrelatedPurchase("pending-token"))
        assertEquals(
            listOf(PendingPurchase("new-invoke", null)),
            store.takeForProducts(listOf("monthly")),
        )
    }

    @Test
    fun `fresh preflight correlates old purchased token before activating a new flow`() {
        val store = PendingPurchaseStore<String, String>()
        val oldPurchase = OwnedPurchaseSnapshot(
            purchaseToken = "old-token",
            productIds = setOf("monthly"),
            state = OwnedPurchaseState.PURCHASED,
        )

        assertEquals(
            PurchasePreparation.READY,
            store.prepareForPurchase("monthly", listOf(oldPurchase)),
        )
        assertTrue(store.hasCorrelatedPurchase("old-token"))
        assertTrue(store.register("monthly", "new-invoke"))
        store.markLaunched("monthly", 200L)
        assertEquals(
            PurchaseUpdateKind.CORRELATED,
            store.routePurchased("old-token", listOf("monthly"), 100L).kind,
        )
        assertEquals(
            PurchaseUpdateKind.ACTIVE,
            store.routePurchased("new-token", listOf("monthly"), 201L).kind,
        )
    }

    @Test
    fun `store clock skew cannot strand a legitimate active callback`() {
        val store = PendingPurchaseStore<String, String>()
        assertTrue(store.register("monthly", "new-invoke"))
        store.updateContext("monthly", "monthly:new-offer")
        store.markLaunched("monthly", 1_000L)

        val routed = store.routePurchased(
            purchaseToken = "unknown-old-token",
            productIds = listOf("monthly"),
            purchaseTimeMillis = 999L,
        )

        assertEquals(PurchaseUpdateKind.ACTIVE, routed.kind)
        assertEquals(PendingPurchase("new-invoke", "monthly:new-offer", 1_000L), routed.activePurchase)
    }

    @Test
    fun `fresh preflight blocks a product that Play still reports pending`() {
        val store = PendingPurchaseStore<String, String>()
        val pendingPurchase = OwnedPurchaseSnapshot(
            purchaseToken = "old-token",
            productIds = setOf("monthly"),
            state = OwnedPurchaseState.PENDING,
        )

        assertEquals(
            PurchasePreparation.PAYMENT_PENDING,
            store.prepareForPurchase("monthly", listOf(pendingPurchase)),
        )
        assertFalse(store.register("monthly", "new-invoke"))
        assertTrue(store.register("annual", "other-invoke"))
    }

    @Test
    fun `product scoped preflight preserves an unrelated deferred purchase`() {
        val store = deferredStore()

        assertEquals(
            PurchasePreparation.READY,
            store.prepareForPurchase("coins", emptyList()),
        )

        assertTrue(store.hasCorrelatedPurchase("pending-token"))
        assertFalse(store.register("monthly", "same-product-invoke"))
        assertTrue(store.register("coins", "other-product-invoke"))
    }

    @Test
    fun `correlation storage is bounded and blocks a flow when full`() {
        val store = PendingPurchaseStore<String, String>()
        val purchases = (1..40).map { index ->
            OwnedPurchaseSnapshot(
                purchaseToken = "token-$index",
                productIds = setOf("monthly"),
                state = OwnedPurchaseState.PURCHASED,
            )
        }

        assertEquals(
            PurchasePreparation.CORRELATION_CAPACITY_EXCEEDED,
            store.prepareForPurchase("monthly", purchases),
        )
        assertFalse(store.register("monthly", "new-invoke"))
    }

    @Test
    fun `preflight cannot consume the reserved correlation slot of an active flow`() {
        val store = PendingPurchaseStore<String, String>()
        assertTrue(store.register("monthly", "active-invoke"))
        val purchases = (1..40).map { index ->
            OwnedPurchaseSnapshot(
                purchaseToken = "token-$index",
                productIds = setOf("monthly"),
                state = OwnedPurchaseState.PURCHASED,
            )
        }

        assertEquals(PurchasePreparation.READY, store.prepareForPurchase("monthly", purchases))
        assertEquals(
            PurchaseUpdateKind.ACTIVE,
            store.routePurchased("active-token", listOf("monthly"), 0L).kind,
        )
    }

    @Test
    fun `only allows one purchase flow at a time`() {
        val store = PendingPurchaseStore<String, String>()

        assertTrue(store.register("monthly", "first"))
        assertFalse(store.register("annual", "second"))
        assertEquals(listOf(PendingPurchase("first", null)), store.drain())
        assertFalse(store.hasPendingPurchase())
    }

    @Test
    fun `ignores context for an unknown product`() {
        val store = PendingPurchaseStore<String, String>()

        store.updateContext("missing", "context")

        assertNull(store.contextFor("missing"))
    }

    private fun deferredStore(): PendingPurchaseStore<String, String> =
        PendingPurchaseStore<String, String>().also { store ->
            assertTrue(store.register("monthly", "invoke"))
            store.updateContext("monthly", "monthly:trial")
            assertEquals(
                PendingPurchase("invoke", "monthly:trial"),
                store.defer(listOf("monthly"), "pending-token"),
            )
        }

    private fun purchase(
        token: String,
        productId: String,
        state: OwnedPurchaseState,
    ) = OwnedPurchaseSnapshot(
        purchaseToken = token,
        productIds = setOf(productId),
        state = state,
    )

    private class FakeLocalTransactionMetadataStore<Context>(
        initial: List<LocalTransactionMetadata<Context>> = emptyList(),
    ) : LocalTransactionMetadataStore<Context> {
        var metadata = initial
            private set
        var replaceSucceeds = true
        var replaceError: RuntimeException? = null

        override fun load(): List<LocalTransactionMetadata<Context>> = metadata

        override fun replace(metadata: List<LocalTransactionMetadata<Context>>): Boolean {
            replaceError?.let { throw it }
            if (!replaceSucceeds) return false
            this.metadata = metadata
            return true
        }
    }

    private class FakeSharedPreferences : SharedPreferences {
        private var values: Map<String, Any?> = emptyMap()
        var commitSucceeds = true

        override fun getAll(): Map<String, *> = values

        override fun getString(key: String, defaultValue: String?): String? =
            values[key] as? String ?: defaultValue

        @Suppress("UNCHECKED_CAST")
        override fun getStringSet(key: String, defaultValues: Set<String>?): Set<String>? =
            values[key] as? Set<String> ?: defaultValues

        override fun getInt(key: String, defaultValue: Int): Int =
            values[key] as? Int ?: defaultValue

        override fun getLong(key: String, defaultValue: Long): Long =
            values[key] as? Long ?: defaultValue

        override fun getFloat(key: String, defaultValue: Float): Float =
            values[key] as? Float ?: defaultValue

        override fun getBoolean(key: String, defaultValue: Boolean): Boolean =
            values[key] as? Boolean ?: defaultValue

        override fun contains(key: String): Boolean = key in values

        override fun edit(): SharedPreferences.Editor = FakeEditor()

        override fun registerOnSharedPreferenceChangeListener(
            listener: SharedPreferences.OnSharedPreferenceChangeListener?,
        ) = Unit

        override fun unregisterOnSharedPreferenceChangeListener(
            listener: SharedPreferences.OnSharedPreferenceChangeListener?,
        ) = Unit

        private inner class FakeEditor : SharedPreferences.Editor {
            private var pending: Map<String, Any?> = values

            override fun putString(key: String, value: String?) = applyValue(key, value)

            override fun putStringSet(key: String, values: Set<String>?) = applyValue(key, values)

            override fun putInt(key: String, value: Int) = applyValue(key, value)

            override fun putLong(key: String, value: Long) = applyValue(key, value)

            override fun putFloat(key: String, value: Float) = applyValue(key, value)

            override fun putBoolean(key: String, value: Boolean) = applyValue(key, value)

            override fun remove(key: String): SharedPreferences.Editor = apply {
                pending = pending - key
            }

            override fun clear(): SharedPreferences.Editor = apply { pending = emptyMap() }

            override fun commit(): Boolean {
                if (commitSucceeds) values = pending
                return commitSucceeds
            }

            override fun apply() {
                commit()
            }

            private fun applyValue(key: String, value: Any?): SharedPreferences.Editor = apply {
                pending = if (value == null) pending - key else pending + (key to value)
            }
        }
    }
}
