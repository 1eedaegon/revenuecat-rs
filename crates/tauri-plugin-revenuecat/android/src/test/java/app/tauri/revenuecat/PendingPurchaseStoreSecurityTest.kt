package app.tauri.revenuecat

import android.content.SharedPreferences
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

class PendingPurchaseStoreSecurityTest {
    @Test
    fun `active callback with blank purchase token fails closed without correlation`() {
        val store = PendingPurchaseStore<String, String>()
        assertTrue(store.register("monthly", "invoke"))
        store.updateContext("monthly", "monthly:trial")

        val routed = store.routePurchased(
            purchaseToken = "   ",
            productIds = listOf("monthly"),
            purchaseTimeMillis = 1L,
        )

        assertEquals(PurchaseUpdateKind.ACTIVE, routed.kind)
        assertFalse(routed.isMetadataDurable)
        assertFalse(assertNotNull(routed.activePurchase).isMetadataDurable)
        assertFalse(store.hasCorrelatedPurchase("   "))
        assertFalse(store.hasPendingPurchase())
    }

    @Test
    fun `pending callback with blank purchase token fails closed without correlation`() {
        val store = PendingPurchaseStore<String, String>()
        assertTrue(store.register("monthly", "invoke"))
        store.updateContext("monthly", "monthly:trial")

        val pending = assertNotNull(
            store.defer(
                productIds = listOf("monthly"),
                purchaseToken = "\t",
            ),
        )

        assertFalse(pending.isMetadataDurable)
        assertFalse(store.hasCorrelatedPurchase("\t"))
        assertFalse(store.hasPendingPurchase())
    }

    @Test
    fun `corrupt unsupported and oversized journals block purchase preparation`() {
        val corruptJournals = listOf(
            "\\u00ZZ=invalid",
            "version=2\ncount=0\n",
            "x".repeat(128 * 1024 + 1),
        )

        corruptJournals.forEach { serialized ->
            val metadataStore = SharedPreferencesLocalTransactionMetadataStore(
                TestSharedPreferences(mapOf(METADATA_KEY to serialized)),
            )
            val store = PendingPurchaseStore<String, NativePurchaseContext>(metadataStore)

            assertTrue(store.hasMetadataPersistenceFailure())
            assertEquals(
                PurchasePreparation.METADATA_PERSISTENCE_FAILED,
                store.prepareForPurchase("monthly", emptyList()),
            )
            assertFalse(store.register("monthly", "invoke"))
        }
    }

    @Test
    fun `oversized journal with valid empty header is corrupt`() {
        val serialized = "version=1\ncount=0\npadding=" +
            "x".repeat(MAX_SERIALIZED_LENGTH)
        val metadataStore = SharedPreferencesLocalTransactionMetadataStore(
            TestSharedPreferences(mapOf(METADATA_KEY to serialized)),
        )
        val store = PendingPurchaseStore<String, NativePurchaseContext>(metadataStore)

        assertTrue(metadataStore.loadWithStatus().isCorrupt)
        assertTrue(store.hasMetadataPersistenceFailure())
        assertEquals(
            PurchasePreparation.METADATA_PERSISTENCE_FAILED,
            store.prepareForPurchase("monthly", emptyList()),
        )
    }

    @Test
    fun `journal over UTF-8 byte limit is corrupt even when character count fits`() {
        val prefix = "version=1\ncount=0\npadding="
        val serialized = prefix + "한".repeat(MAX_SERIALIZED_LENGTH - prefix.length)
        assertTrue(serialized.length <= MAX_SERIALIZED_LENGTH)
        assertTrue(serialized.toByteArray(Charsets.UTF_8).size > MAX_SERIALIZED_LENGTH)
        val metadataStore = SharedPreferencesLocalTransactionMetadataStore(
            TestSharedPreferences(mapOf(METADATA_KEY to serialized)),
        )

        assertTrue(metadataStore.loadWithStatus().isCorrupt)
    }

    @Test
    fun `structurally invalid journals block purchase preparation`() {
        val invalidJournals = listOf(
            "version=1\ncount=33\n",
            "version=1\ncount=1\n",
            """
                version=1
                count=1
                transaction.0.purchaseToken=purchase-token
                transaction.0.productCount=1
                transaction.0.product.0=monthly
                transaction.0.hasContext=true
            """.trimIndent(),
        )

        invalidJournals.forEach { serialized ->
            val metadataStore = SharedPreferencesLocalTransactionMetadataStore(
                TestSharedPreferences(mapOf(METADATA_KEY to serialized)),
            )
            val loadResult = metadataStore.loadWithStatus()
            val store = PendingPurchaseStore<String, NativePurchaseContext>(metadataStore)

            assertTrue(loadResult.isCorrupt)
            assertTrue(store.hasMetadataPersistenceFailure())
            assertEquals(
                PurchasePreparation.METADATA_PERSISTENCE_FAILED,
                store.prepareForPurchase("monthly", emptyList()),
            )
        }
    }

    @Test
    fun `invalid live metadata replacement fails without changing valid journal`() {
        val preferences = TestSharedPreferences()
        val store = SharedPreferencesLocalTransactionMetadataStore(preferences)
        val valid = validMetadata()
        assertTrue(store.replace(listOf(valid)))
        val before = preferences.getString(METADATA_KEY, null)

        val replaced = store.replace(
            listOf(valid.copy(purchaseToken = "  ")),
        )

        assertFalse(replaced)
        assertEquals(before, preferences.getString(METADATA_KEY, null))
        assertEquals(listOf(valid), store.load())
    }

    @Test
    fun `invalid live context replacement fails without changing valid journal`() {
        val preferences = TestSharedPreferences()
        val store = SharedPreferencesLocalTransactionMetadataStore(preferences)
        val valid = validMetadata()
        assertTrue(store.replace(listOf(valid)))
        val before = preferences.getString(METADATA_KEY, null)
        val invalidReceipt = NativeReceiptContext(
            presentedOfferingIdentifier = null,
            presentedPlacementIdentifier = null,
            appliedTargetingRule = null,
            priceMicros = -1L,
            currency = "USD",
            normalDuration = null,
            sdkOriginated = true,
        )
        val invalid = valid.copy(
            context = valid.context?.copy(receiptContext = invalidReceipt),
        )

        val replaced = store.replace(listOf(invalid))

        assertFalse(replaced)
        assertEquals(before, preferences.getString(METADATA_KEY, null))
        assertEquals(listOf(valid), store.load())
    }

    @Test
    fun `oversized live replacement fails without truncating or changing valid journal`() {
        val preferences = TestSharedPreferences()
        val store = SharedPreferencesLocalTransactionMetadataStore(preferences)
        val valid = validMetadata()
        assertTrue(store.replace(listOf(valid)))
        val before = preferences.getString(METADATA_KEY, null)
        val oversized = (0..32).map { index ->
            valid.copy(purchaseToken = "purchase-token-$index")
        }

        val replaced = store.replace(oversized)

        assertFalse(replaced)
        assertEquals(before, preferences.getString(METADATA_KEY, null))
        assertEquals(listOf(valid), store.load())
    }

    @Test
    fun `encoded journal over byte limit fails without changing valid journal`() {
        val preferences = TestSharedPreferences()
        val store = SharedPreferencesLocalTransactionMetadataStore(preferences)
        val valid = validMetadata()
        assertTrue(store.replace(listOf(valid)))
        val before = preferences.getString(METADATA_KEY, null)
        val oversizedEncoding = (0 until 17).map { index ->
            val suffix = index.toString()
            valid.copy(
                purchaseToken = "x".repeat(MAX_TOKEN_LENGTH - suffix.length) + suffix,
            )
        }

        val replaced = store.replace(oversizedEncoding)

        assertFalse(replaced)
        assertEquals(before, preferences.getString(METADATA_KEY, null))
        assertEquals(listOf(valid), store.load())
    }

    @Test
    fun `multibyte encoded journal over byte limit preserves valid journal`() {
        val preferences = TestSharedPreferences()
        val store = SharedPreferencesLocalTransactionMetadataStore(preferences)
        val valid = validMetadata()
        assertTrue(store.replace(listOf(valid)))
        val before = preferences.getString(METADATA_KEY, null)
        val oversizedEncoding = (0 until MULTIBYTE_TRANSACTION_COUNT).map { index ->
            val suffix = index.toString()
            valid.copy(
                purchaseToken = "한".repeat(MULTIBYTE_TOKEN_LENGTH - suffix.length) + suffix,
            )
        }

        val replaced = store.replace(oversizedEncoding)

        assertFalse(replaced)
        assertEquals(before, preferences.getString(METADATA_KEY, null))
        assertEquals(listOf(valid), store.load())
    }

    private fun validMetadata() = LocalTransactionMetadata(
        purchaseToken = "purchase-token",
        productIds = setOf("monthly"),
        context = NativePurchaseContext(
            productId = "monthly",
            basePlanId = "monthly-base",
            offerId = null,
            receiptContext = null,
        ),
    )

    private companion object {
        const val METADATA_KEY = "play_transactions_v1"
        const val MAX_SERIALIZED_LENGTH = 128 * 1024
        const val MAX_TOKEN_LENGTH = 8 * 1024
        const val MULTIBYTE_TOKEN_LENGTH = 6_000
        const val MULTIBYTE_TRANSACTION_COUNT = 8
    }

    private class TestSharedPreferences(
        initialValues: Map<String, Any?> = emptyMap(),
    ) : SharedPreferences {
        private var values: Map<String, Any?> = initialValues

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

        override fun edit(): SharedPreferences.Editor = TestEditor()

        override fun registerOnSharedPreferenceChangeListener(
            listener: SharedPreferences.OnSharedPreferenceChangeListener?,
        ) = Unit

        override fun unregisterOnSharedPreferenceChangeListener(
            listener: SharedPreferences.OnSharedPreferenceChangeListener?,
        ) = Unit

        private inner class TestEditor : SharedPreferences.Editor {
            private var pending: Map<String, Any?> = values

            override fun putString(key: String, value: String?) = set(key, value)
            override fun putStringSet(key: String, values: Set<String>?) = set(key, values)
            override fun putInt(key: String, value: Int) = set(key, value)
            override fun putLong(key: String, value: Long) = set(key, value)
            override fun putFloat(key: String, value: Float) = set(key, value)
            override fun putBoolean(key: String, value: Boolean) = set(key, value)
            override fun remove(key: String): SharedPreferences.Editor = apply { pending -= key }
            override fun clear(): SharedPreferences.Editor = apply { pending = emptyMap() }

            override fun commit(): Boolean {
                values = pending
                return true
            }

            override fun apply() {
                commit()
            }

            private fun set(key: String, value: Any?): SharedPreferences.Editor = apply {
                pending = if (value == null) pending - key else pending + (key to value)
            }
        }
    }
}
