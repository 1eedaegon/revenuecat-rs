package app.tauri.revenuecat

import android.annotation.SuppressLint
import android.content.Context
import android.content.SharedPreferences
import java.io.StringReader
import java.io.StringWriter
import java.util.Properties

private const val MAX_PERSISTED_TRANSACTIONS = 32
private const val MAX_PERSISTED_IDENTIFIER_LENGTH = 512

internal fun localTransactionMetadataStore(
    context: Context,
): LocalTransactionMetadataStore<NativePurchaseContext> =
    SharedPreferencesLocalTransactionMetadataStore(
        context.applicationContext.getSharedPreferences(
            "app.tauri.revenuecat.transaction_metadata",
            Context.MODE_PRIVATE,
        ),
    )

internal class SharedPreferencesLocalTransactionMetadataStore(
    private val preferences: SharedPreferences,
) : LocalTransactionMetadataStore<NativePurchaseContext> {
    private companion object {
        const val METADATA_KEY = "play_transactions_v1"
        const val SCHEMA_VERSION = 1
        const val MAX_SERIALIZED_LENGTH = 128 * 1024
        const val MAX_TOKEN_LENGTH = 8 * 1024
        const val MAX_PRODUCTS_PER_TRANSACTION = 32
    }

    @Synchronized
    override fun load(): List<LocalTransactionMetadata<NativePurchaseContext>> =
        loadWithStatus().metadata

    @Synchronized
    override fun loadWithStatus(): LocalTransactionMetadataLoadResult<NativePurchaseContext> {
        val serialized = runCatching { preferences.getString(METADATA_KEY, null) }
            .getOrElse {
                return LocalTransactionMetadataLoadResult(emptyList(), isCorrupt = true)
            } ?: return LocalTransactionMetadataLoadResult(emptyList(), isCorrupt = false)
        if (serialized.exceedsByteLimit()) {
            return LocalTransactionMetadataLoadResult(emptyList(), isCorrupt = true)
        }
        return runCatching {
            LocalTransactionMetadataLoadResult(
                metadata = decodeMetadata(serialized),
                isCorrupt = false,
            )
        }.getOrElse {
            LocalTransactionMetadataLoadResult(emptyList(), isCorrupt = true)
        }
    }

    // Synchronous commit closes the process-death window before a purchase callback succeeds.
    @SuppressLint("UseKtx")
    @Synchronized
    override fun replace(
        metadata: List<LocalTransactionMetadata<NativePurchaseContext>>,
    ): Boolean {
        if (metadata.size > MAX_PERSISTED_TRANSACTIONS) return false
        val validated = metadata.map(::validatedMetadata)
        if (validated.any { it == null }) return false
        val retained = validated.filterNotNull()
        val serialized = encodeMetadata(retained)
        if (serialized.exceedsByteLimit()) return false
        return preferences.edit().putString(METADATA_KEY, serialized).commit()
    }

    private fun String.exceedsByteLimit(): Boolean =
        length > MAX_SERIALIZED_LENGTH ||
            toByteArray(Charsets.UTF_8).size > MAX_SERIALIZED_LENGTH

    private fun decodeMetadata(
        serialized: String,
    ): List<LocalTransactionMetadata<NativePurchaseContext>> {
        val properties = Properties().apply { load(StringReader(serialized)) }
        require(properties.getProperty("version")?.toIntOrNull() == SCHEMA_VERSION)
        val count = requireNotNull(properties.getProperty("count")?.toIntOrNull())
        require(count in 0..MAX_PERSISTED_TRANSACTIONS)
        return (0 until count)
            .map { index -> requireNotNull(properties.toMetadata(index)) }
    }

    private fun encodeMetadata(
        metadata: List<LocalTransactionMetadata<NativePurchaseContext>>,
    ): String {
        val properties = Properties().apply {
            setProperty("version", SCHEMA_VERSION.toString())
            setProperty("count", metadata.size.toString())
        }
        metadata.forEachIndexed { index, transaction ->
            properties.writeMetadata(index, transaction)
        }
        return StringWriter().also { properties.store(it, null) }.toString()
    }

    private fun validatedMetadata(
        metadata: LocalTransactionMetadata<NativePurchaseContext>,
    ): LocalTransactionMetadata<NativePurchaseContext>? {
        val productIds = metadata.productIds
        if (!metadata.purchaseToken.isSafeToken() ||
            productIds.size !in 1..MAX_PRODUCTS_PER_TRANSACTION ||
            productIds.any { !it.isSafeIdentifier() }
        ) {
            return null
        }
        if (metadata.context?.isValidFor(productIds) == false) return null
        return metadata
    }

    private fun Properties.writeMetadata(
        index: Int,
        metadata: LocalTransactionMetadata<NativePurchaseContext>,
    ) {
        val prefix = "transaction.$index"
        setProperty("$prefix.purchaseToken", metadata.purchaseToken)
        setProperty("$prefix.productCount", metadata.productIds.size.toString())
        metadata.productIds.forEachIndexed { productIndex, productId ->
            setProperty("$prefix.product.$productIndex", productId)
        }
        setProperty("$prefix.hasContext", (metadata.context != null).toString())
        metadata.context?.let { writeContext(prefix, it) }
    }

    private fun Properties.writeContext(prefix: String, context: NativePurchaseContext) {
        setProperty("$prefix.context.productId", context.productId)
        setOptional("$prefix.context.basePlanId", context.basePlanId)
        setOptional("$prefix.context.offerId", context.offerId)
        setProperty("$prefix.context.hasReceipt", (context.receiptContext != null).toString())
        context.receiptContext?.let { writeReceiptContext(prefix, it) }
    }

    private fun Properties.writeReceiptContext(prefix: String, context: NativeReceiptContext) {
        val receiptPrefix = "$prefix.context.receipt"
        setOptional("$receiptPrefix.offering", context.presentedOfferingIdentifier)
        setOptional("$receiptPrefix.placement", context.presentedPlacementIdentifier)
        setOptional("$receiptPrefix.priceMicros", context.priceMicros?.toString())
        setOptional("$receiptPrefix.currency", context.currency)
        setOptional("$receiptPrefix.normalDuration", context.normalDuration)
        setProperty("$receiptPrefix.sdkOriginated", context.sdkOriginated.toString())
        setProperty("$receiptPrefix.hasTargeting", (context.appliedTargetingRule != null).toString())
        context.appliedTargetingRule?.let { rule ->
            setProperty("$receiptPrefix.ruleId", rule.ruleId)
            setProperty("$receiptPrefix.revision", rule.revision.toString())
        }
    }

    private fun Properties.toMetadata(
        index: Int,
    ): LocalTransactionMetadata<NativePurchaseContext>? {
        val prefix = "transaction.$index"
        val purchaseToken = getProperty("$prefix.purchaseToken")
            ?.takeIf { it.isSafeToken() } ?: return null
        val productCount = getProperty("$prefix.productCount")?.toIntOrNull() ?: return null
        if (productCount !in 1..MAX_PRODUCTS_PER_TRANSACTION) return null
        val productIds = (0 until productCount)
            .mapNotNull { getProperty("$prefix.product.$it") }
            .toSet()
        if (productIds.size != productCount || productIds.any { !it.isSafeIdentifier() }) return null
        val hasContext = getProperty("$prefix.hasContext")?.toBooleanStrictOrNull() ?: return null
        val context = if (hasContext) {
            runCatching { toPurchaseContext(prefix) }.getOrNull()
                ?.takeIf { it.isValidFor(productIds) } ?: return null
        } else {
            null
        }
        return LocalTransactionMetadata(purchaseToken, productIds, context)
    }

    private fun Properties.toPurchaseContext(prefix: String): NativePurchaseContext {
        val contextPrefix = "$prefix.context"
        val hasReceipt = requireNotNull(
            getProperty("$contextPrefix.hasReceipt")?.toBooleanStrictOrNull(),
        )
        val receiptContext = if (hasReceipt) toReceiptContext(contextPrefix) else null
        return NativePurchaseContext(
            productId = requireNotNull(getProperty("$contextPrefix.productId")),
            basePlanId = getProperty("$contextPrefix.basePlanId"),
            offerId = getProperty("$contextPrefix.offerId"),
            receiptContext = receiptContext,
        )
    }

    private fun Properties.toReceiptContext(contextPrefix: String): NativeReceiptContext {
        val prefix = "$contextPrefix.receipt"
        val hasTargeting = requireNotNull(
            getProperty("$prefix.hasTargeting")?.toBooleanStrictOrNull(),
        )
        val targeting = if (hasTargeting) {
            ReceiptTargetingRule(
                ruleId = requireNotNull(getProperty("$prefix.ruleId")),
                revision = requireNotNull(getProperty("$prefix.revision")?.toLongOrNull()),
            )
        } else {
            null
        }
        return NativeReceiptContext(
            presentedOfferingIdentifier = getProperty("$prefix.offering"),
            presentedPlacementIdentifier = getProperty("$prefix.placement"),
            appliedTargetingRule = targeting,
            priceMicros = getProperty("$prefix.priceMicros")?.let {
                requireNotNull(it.toLongOrNull())
            },
            currency = getProperty("$prefix.currency"),
            normalDuration = getProperty("$prefix.normalDuration"),
            sdkOriginated = requireNotNull(
                getProperty("$prefix.sdkOriginated")?.toBooleanStrictOrNull(),
            ),
        )
    }

    private fun Properties.setOptional(key: String, value: String?) {
        value?.let { setProperty(key, it) }
    }

    private fun NativePurchaseContext.isValidFor(productIds: Set<String>): Boolean =
        productId.isSafeIdentifier() &&
            productId in productIds &&
            basePlanId.isSafeOptionalIdentifier() &&
            offerId.isSafeOptionalIdentifier() &&
            (offerId == null || basePlanId != null) &&
            (basePlanId != null || receiptContext != null) &&
            (receiptContext?.isValidForPersistence() != false)

    private fun NativeReceiptContext.isValidForPersistence(): Boolean {
        val optionalIdentifiers = listOf(
            presentedOfferingIdentifier,
            presentedPlacementIdentifier,
            currency,
            normalDuration,
        )
        val isTargetingValid = appliedTargetingRule?.ruleId.isSafeOptionalIdentifier()
        val isPriceComplete = (priceMicros == null) == (currency == null)
        val isPriceValid = priceMicros == null || priceMicros >= 0
        return optionalIdentifiers.all { it.isSafeOptionalIdentifier() } &&
            isTargetingValid && isPriceComplete && isPriceValid
    }

    private fun String.isSafeToken(): Boolean = isNotBlank() && length <= MAX_TOKEN_LENGTH

    private fun String.isSafeIdentifier(): Boolean =
        isNotBlank() && length <= MAX_PERSISTED_IDENTIFIER_LENGTH

    private fun String?.isSafeOptionalIdentifier(): Boolean =
        this == null || isSafeIdentifier()
}
