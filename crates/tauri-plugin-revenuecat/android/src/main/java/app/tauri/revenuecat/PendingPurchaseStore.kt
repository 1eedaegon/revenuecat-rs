package app.tauri.revenuecat

import app.tauri.annotation.InvokeArg
import org.json.JSONObject

private const val MAX_CORRELATED_PURCHASES = 32
private const val MAX_METADATA_IDENTIFIER_LENGTH = 512

internal data class PendingPurchase<out Callback, out Context>(
    val callback: Callback,
    val context: Context?,
    val launchedAtMillis: Long? = null,
    val isMetadataDurable: Boolean = true,
)

internal enum class OwnedPurchaseState {
    PENDING,
    PURCHASED,
}

internal data class OwnedPurchaseSnapshot(
    val purchaseToken: String,
    val productIds: Set<String>,
    val state: OwnedPurchaseState,
)

internal enum class PurchasePreparation {
    READY,
    PAYMENT_PENDING,
    CORRELATION_CAPACITY_EXCEEDED,
    METADATA_PERSISTENCE_FAILED,
}

internal enum class PurchaseUpdateKind {
    ACTIVE,
    CORRELATED,
    OUT_OF_BAND,
}

internal data class RoutedPurchase<out Callback, out Context>(
    val kind: PurchaseUpdateKind,
    val activePurchase: PendingPurchase<Callback, Context>? = null,
    val isMetadataDurable: Boolean = true,
)

internal data class ReceiptTargetingRule(
    val ruleId: String,
    val revision: Long,
)

internal data class NativeReceiptContext(
    val presentedOfferingIdentifier: String?,
    val presentedPlacementIdentifier: String?,
    val appliedTargetingRule: ReceiptTargetingRule?,
    val priceMicros: Long?,
    val currency: String?,
    val normalDuration: String?,
    val sdkOriginated: Boolean,
)

@InvokeArg
internal class ReceiptTargetingRuleArgs {
    var ruleId: String = ""
    var revision: Long = 0
}

@InvokeArg
internal class NativeReceiptContextArgs {
    var presentedOfferingIdentifier: String? = null
    var presentedPlacementIdentifier: String? = null
    var appliedTargetingRule: ReceiptTargetingRuleArgs? = null
    var priceMicros: Long? = null
    var currency: String? = null
    var normalDuration: String? = null
    var sdkOriginated: Boolean = false
}

internal fun NativeReceiptContextArgs.toValidatedReceiptContext(): NativeReceiptContext? =
    NativeReceiptContext(
        presentedOfferingIdentifier = presentedOfferingIdentifier,
        presentedPlacementIdentifier = presentedPlacementIdentifier,
        appliedTargetingRule = appliedTargetingRule?.let {
            ReceiptTargetingRule(it.ruleId, it.revision)
        },
        priceMicros = priceMicros,
        currency = currency,
        normalDuration = normalDuration,
        sdkOriginated = sdkOriginated,
    ).takeIf(NativeReceiptContext::isValid)

internal fun NativeReceiptContext.toJsonObject(): JSONObject = JSONObject()
    .put("sdkOriginated", sdkOriginated)
    .apply {
        presentedOfferingIdentifier?.let { put("presentedOfferingIdentifier", it) }
        presentedPlacementIdentifier?.let { put("presentedPlacementIdentifier", it) }
        appliedTargetingRule?.let { rule ->
            put(
                "appliedTargetingRule",
                JSONObject().put("ruleId", rule.ruleId).put("revision", rule.revision),
            )
        }
        priceMicros?.let { put("priceMicros", it) }
        currency?.let { put("currency", it) }
        normalDuration?.let { put("normalDuration", it) }
    }

private fun NativeReceiptContext.isValid(): Boolean {
    val optionalText = listOf(
        presentedOfferingIdentifier,
        presentedPlacementIdentifier,
        currency,
        normalDuration,
    )
    val isTargetingValid = appliedTargetingRule?.ruleId
        ?.let { it.isNotBlank() && it.length <= MAX_METADATA_IDENTIFIER_LENGTH } != false
    val isPriceComplete = (priceMicros == null) == (currency == null)
    val isPriceValid = priceMicros == null || priceMicros >= 0
    return optionalText.filterNotNull().none {
        it.isBlank() || it.length > MAX_METADATA_IDENTIFIER_LENGTH
    } &&
        isTargetingValid && isPriceComplete && isPriceValid
}

internal data class NativePurchaseContext(
    val productId: String,
    val basePlanId: String?,
    val offerId: String?,
    val receiptContext: NativeReceiptContext?,
)

internal data class LocalTransactionMetadata<out Context>(
    val purchaseToken: String,
    val productIds: Set<String>,
    val context: Context?,
)

internal data class LocalTransactionMetadataLoadResult<out Context>(
    val metadata: List<LocalTransactionMetadata<Context>>,
    val isCorrupt: Boolean,
)

internal interface LocalTransactionMetadataStore<Context> {
    fun load(): List<LocalTransactionMetadata<Context>>

    fun loadWithStatus(): LocalTransactionMetadataLoadResult<Context> = runCatching {
        LocalTransactionMetadataLoadResult(load(), isCorrupt = false)
    }.getOrElse {
        LocalTransactionMetadataLoadResult(emptyList(), isCorrupt = true)
    }

    fun replace(metadata: List<LocalTransactionMetadata<Context>>): Boolean
}

private class EmptyLocalTransactionMetadataStore<Context> :
    LocalTransactionMetadataStore<Context> {
    override fun load(): List<LocalTransactionMetadata<Context>> = emptyList()

    override fun replace(metadata: List<LocalTransactionMetadata<Context>>): Boolean = true
}

private data class CorrelatedPurchase<out Context>(
    val purchaseToken: String,
    val productIds: Set<String>,
    val context: Context?,
    val isDurable: Boolean,
)

internal class PendingPurchaseStore<Callback, Context>(
    private val metadataStore: LocalTransactionMetadataStore<Context> =
        EmptyLocalTransactionMetadataStore(),
) {
    private var purchases: Map<String, PendingPurchase<Callback, Context>> = emptyMap()
    private var deferredPurchases: Map<String, CorrelatedPurchase<Context>> = emptyMap()
    private var metadataPersistenceFailed = false
    private var completedCorrelations: Map<String, CorrelatedPurchase<Context>> =
        restoreDurableCorrelations()

    @Synchronized
    fun register(productId: String, callback: Callback): Boolean {
        val isSameProductDeferred = deferredPurchases.values.any { productId in it.productIds }
        val isCorrelationCapacityFull =
            deferredPurchases.size + completedCorrelations.size >= MAX_CORRELATED_PURCHASES
        if (metadataPersistenceFailed || purchases.isNotEmpty() || isSameProductDeferred ||
            isCorrelationCapacityFull
        ) {
            return false
        }
        purchases = mapOf(productId to PendingPurchase(callback, null))
        return true
    }

    @Synchronized
    fun updateContext(productId: String, context: Context) {
        val pending = purchases[productId] ?: return
        purchases = purchases + (productId to pending.copy(context = context))
    }

    @Synchronized
    fun markLaunched(productId: String, launchedAtMillis: Long) {
        val pending = purchases[productId] ?: return
        purchases = purchases +
            (productId to pending.copy(launchedAtMillis = launchedAtMillis))
    }

    /**
     * Releases the active callback after Play reports PENDING, while retaining the immutable
     * purchase-token correlation needed to serialize a later PURCHASED query safely.
     *
     * Correlations are token-keyed and bounded. Only the same deferred product is blocked; an
     * unrelated product may still start after the PENDING callback has been released.
     */
    @Synchronized
    fun defer(productIds: List<String>, purchaseToken: String): PendingPurchase<Callback, Context>? {
        val productIdSet = productIds.toSet()
        val active = purchases.entries.firstOrNull { it.key in productIdSet } ?: return null
        purchases = purchases - active.key
        if (purchaseToken.isBlank()) {
            return active.value.copy(isMetadataDurable = false)
        }
        val existing = deferredPurchases[purchaseToken] ?: completedCorrelations[purchaseToken]
        val correlation = if (existing == null) {
            CorrelatedPurchase(
                purchaseToken = purchaseToken,
                productIds = productIdSet,
                context = active.value.context.takeIf { purchaseToken.isNotBlank() },
                isDurable = active.value.context != null && purchaseToken.isNotBlank(),
            )
        } else {
            // Play tokens are expected to be unique. If they are not, retain a conservative
            // product union but discard option context so it can never be misattributed.
            CorrelatedPurchase(
                purchaseToken = purchaseToken,
                productIds = existing.productIds + productIdSet,
                context = null,
                isDurable = existing.isDurable || active.value.context != null,
            )
        }
        deferredPurchases = deferredPurchases + (purchaseToken to correlation)
        completedCorrelations = completedCorrelations - purchaseToken
        val isMetadataDurable = !correlation.isDurable || persistDurableCorrelations()
        return active.value.copy(isMetadataDurable = isMetadataDurable)
    }

    @Synchronized
    fun hasCorrelatedPurchase(purchaseToken: String): Boolean =
        purchaseToken in deferredPurchases || purchaseToken in completedCorrelations

    /**
     * Reconciles and records Play-owned tokens before registering a new flow. This makes stale
     * PURCHASED callbacks distinguishable even after the in-memory state has been recreated.
     */
    @Synchronized
    fun prepareForPurchase(
        productId: String,
        ownedPurchases: List<OwnedPurchaseSnapshot>,
    ): PurchasePreparation {
        // The active flow already reserved the one remaining correlation
        // slot when it registered. A concurrent/direct preflight must not
        // fill the bounded store before that flow receives its token.
        if (purchases.isNotEmpty()) return PurchasePreparation.READY
        val relevantByToken = ownedPurchases
            .filter { productId in it.productIds }
            .groupBy(OwnedPurchaseSnapshot::purchaseToken)
        // Purchase preflight queries only the current product's Play product type. Reconcile
        // correlations for this product while retaining unrelated deferred/completed tokens that
        // belong to the other product type and were intentionally absent from this snapshot.
        var nextDeferred = deferredPurchases.filterValues { productId !in it.productIds }
        var nextCompleted = completedCorrelations.filterValues { productId !in it.productIds }
        relevantByToken.forEach { (token, observations) ->
            val observed = preferredObservation(observations) ?: return@forEach
            val existing = deferredPurchases[token] ?: completedCorrelations[token]
            val correlation = existing
                ?.validatedAgainst(observed)
                ?: CorrelatedPurchase(token, observed.productIds, null, false)
            if (observed.state == OwnedPurchaseState.PENDING) {
                nextDeferred = nextDeferred + (token to correlation)
                nextCompleted = nextCompleted - token
            } else {
                nextCompleted = nextCompleted + (token to correlation)
                nextDeferred = nextDeferred - token
            }
        }
        durableCorrelations().forEach { (token, correlation) ->
            if (token !in nextDeferred && token !in nextCompleted) {
                nextCompleted = nextCompleted + (token to correlation)
            }
        }
        val correlationCount = (nextDeferred.keys + nextCompleted.keys).size
        if (correlationCount >= MAX_CORRELATED_PURCHASES) {
            val durableTokens = (nextDeferred + nextCompleted)
                .filterValues(CorrelatedPurchase<Context>::isDurable)
                .keys
            val retainedTokens = (durableTokens + nextDeferred.keys + nextCompleted.keys)
                .take(MAX_CORRELATED_PURCHASES).toSet()
            deferredPurchases = nextDeferred.filterKeys(retainedTokens::contains)
            completedCorrelations = nextCompleted.filterKeys(retainedTokens::contains)
            if (!persistDurableCorrelations()) {
                return PurchasePreparation.METADATA_PERSISTENCE_FAILED
            }
            return PurchasePreparation.CORRELATION_CAPACITY_EXCEEDED
        }
        deferredPurchases = nextDeferred
        completedCorrelations = nextCompleted
        if (!persistDurableCorrelations()) {
            return PurchasePreparation.METADATA_PERSISTENCE_FAILED
        }
        return if (nextDeferred.values.any { productId in it.productIds }) {
            PurchasePreparation.PAYMENT_PENDING
        } else {
            PurchasePreparation.READY
        }
    }

    /** Routes PURCHASED token-first, so an older transaction cannot consume a newer callback. */
    @Suppress("UNUSED_PARAMETER")
    @Synchronized
    fun routePurchased(
        purchaseToken: String,
        productIds: List<String>,
        purchaseTimeMillis: Long,
    ): RoutedPurchase<Callback, Context> {
        val productIdSet = productIds.toSet()
        if (purchaseToken.isBlank()) {
            val active = purchases.entries.firstOrNull { it.key in productIdSet }
                ?: return RoutedPurchase(PurchaseUpdateKind.OUT_OF_BAND)
            purchases = purchases - active.key
            return RoutedPurchase(
                kind = PurchaseUpdateKind.ACTIVE,
                activePurchase = active.value.copy(isMetadataDurable = false),
                isMetadataDurable = false,
            )
        }
        val existing = deferredPurchases[purchaseToken] ?: completedCorrelations[purchaseToken]
        if (existing != null) {
            val validated = existing.validatedAgainst(
                OwnedPurchaseSnapshot(
                    purchaseToken = purchaseToken,
                    productIds = productIds.toSet(),
                    state = OwnedPurchaseState.PURCHASED,
                ),
            )
            if (purchaseToken in deferredPurchases) {
                deferredPurchases = deferredPurchases + (purchaseToken to validated)
            } else {
                completedCorrelations = completedCorrelations + (purchaseToken to validated)
            }
            val isMetadataDurable = !validated.isDurable || persistDurableCorrelations()
            return RoutedPurchase(
                kind = PurchaseUpdateKind.CORRELATED,
                isMetadataDurable = isMetadataDurable,
            )
        }
        val active = purchases.entries.firstOrNull { it.key in productIdSet }
            ?: return RoutedPurchase(PurchaseUpdateKind.OUT_OF_BAND)
        // Play's purchaseTime and the device clock captured at launch are not
        // guaranteed to be comparable. Preflight already token-correlated
        // every owned purchase, so a novel matching token belongs to the
        // active flow regardless of clock skew.
        purchases = purchases - active.key
        completedCorrelations = completedCorrelations + (
            purchaseToken to CorrelatedPurchase(
                purchaseToken = purchaseToken,
                productIds = productIdSet,
                context = active.value.context.takeIf { purchaseToken.isNotBlank() },
                isDurable = active.value.context != null && purchaseToken.isNotBlank(),
            )
        )
        val correlation = completedCorrelations.getValue(purchaseToken)
        val isMetadataDurable = !correlation.isDurable || persistDurableCorrelations()
        return RoutedPurchase(
            kind = PurchaseUpdateKind.ACTIVE,
            activePurchase = active.value.copy(isMetadataDurable = isMetadataDurable),
            isMetadataDurable = isMetadataDurable,
        )
    }

    /**
     * Reconciles the deferred marker against one complete Play ownership query.
     *
     * Context is returned only when both the immutable purchase token and the complete product-id
     * set match. A token match with different products is still enough to clear a completed marker,
     * but never enough to claim a base plan or offer.
     */
    @Synchronized
    fun reconcileOwnedPurchases(
        ownedPurchases: List<OwnedPurchaseSnapshot>,
    ): Map<String, Context?> = reconcile(ownedPurchases)

    /**
     * Transactionally clears the exact token before Play finalization.
     *
     * Rust invokes this path only after its backend accepted the receipt. Persisting the deletion
     * first prevents an acknowledged/consumed token from becoming an orphaned journal entry if the
     * process dies immediately after the Play call.
     */
    @Synchronized
    fun removeCorrelationBeforeStoreFinish(purchaseToken: String): Boolean {
        val removed = deferredPurchases[purchaseToken] ?: completedCorrelations[purchaseToken]
        val nextDeferred = deferredPurchases - purchaseToken
        val nextCompleted = completedCorrelations - purchaseToken
        if (removed?.isDurable == true &&
            !persistDurableCorrelations(nextDeferred, nextCompleted, force = true)
        ) {
            return false
        }
        deferredPurchases = nextDeferred
        completedCorrelations = nextCompleted
        return true
    }

    @Synchronized
    fun takeForProducts(productIds: List<String>): List<PendingPurchase<Callback, Context>> {
        val completedProductIds = productIds.toSet()
        val completed = purchases
            .filterKeys(completedProductIds::contains)
            .values
            .toList()
        purchases = purchases - completedProductIds
        return completed
    }

    @Synchronized
    fun contextFor(productId: String): Context? = purchases[productId]?.context

    @Synchronized
    fun hasPendingPurchase(): Boolean = purchases.isNotEmpty() || deferredPurchases.isNotEmpty()

    @Synchronized
    fun hasDeferredPurchase(): Boolean = deferredPurchases.isNotEmpty()

    @Synchronized
    fun hasMetadataPersistenceFailure(): Boolean = metadataPersistenceFailed

    @Synchronized
    fun remove(productId: String): PendingPurchase<Callback, Context>? {
        val removed = purchases[productId]
        purchases = purchases - productId
        return removed
    }

    @Synchronized
    fun drain(): List<PendingPurchase<Callback, Context>> {
        val pending = purchases.values.toList()
        purchases = emptyMap()
        return pending
    }

    private fun reconcile(ownedPurchases: List<OwnedPurchaseSnapshot>): Map<String, Context?> {
        val ownedByToken = ownedPurchases.groupBy(OwnedPurchaseSnapshot::purchaseToken)
        var nextDeferred: Map<String, CorrelatedPurchase<Context>> = emptyMap()
        // Completed purchases remain retryable until backend success causes
        // finishTransaction to remove their token explicitly. A transient
        // empty Play query must not erase their base-plan/offer context.
        var nextCompleted = completedCorrelations
        var completedContexts: Map<String, Context?> = emptyMap()

        deferredPurchases.forEach { (token, correlation) ->
            val observed = preferredObservation(ownedByToken[token].orEmpty())
            when (observed?.state) {
                OwnedPurchaseState.PENDING -> {
                    nextDeferred = nextDeferred + (token to correlation.validatedAgainst(observed))
                }

                OwnedPurchaseState.PURCHASED -> {
                    val validated = correlation.validatedAgainst(observed)
                    completedContexts = completedContexts + (token to validated.context)
                    nextCompleted = nextCompleted + (token to validated)
                }

                null -> if (correlation.isDurable) {
                    nextCompleted = nextCompleted + (token to correlation)
                } else {
                    Unit
                }
            }
        }

        completedCorrelations.forEach { (token, correlation) ->
            val observed = preferredObservation(ownedByToken[token].orEmpty())
            when (observed?.state) {
                OwnedPurchaseState.PENDING -> {
                    val validated = correlation.validatedAgainst(observed)
                    nextDeferred = nextDeferred + (token to validated)
                    nextCompleted = nextCompleted - token
                }

                OwnedPurchaseState.PURCHASED -> {
                    val validated = correlation.validatedAgainst(observed)
                    completedContexts = completedContexts + (token to validated.context)
                    nextCompleted = nextCompleted + (token to validated)
                }

                null -> Unit
            }
        }

        deferredPurchases = nextDeferred
        completedCorrelations = nextCompleted
        persistDurableCorrelations()
        return completedContexts
    }

    private fun preferredObservation(
        observations: List<OwnedPurchaseSnapshot>,
    ): OwnedPurchaseSnapshot? = observations.firstOrNull {
        it.state == OwnedPurchaseState.PURCHASED
    } ?: observations.firstOrNull()

    private fun CorrelatedPurchase<Context>.validatedAgainst(
        observed: OwnedPurchaseSnapshot,
    ): CorrelatedPurchase<Context> {
        val isIdentityProven = purchaseToken.isNotBlank() &&
            purchaseToken == observed.purchaseToken &&
            productIds == observed.productIds
        return copy(context = context.takeIf { isIdentityProven })
    }

    private fun restoreDurableCorrelations(): Map<String, CorrelatedPurchase<Context>> {
        val loadResult = runCatching(metadataStore::loadWithStatus).getOrElse {
            LocalTransactionMetadataLoadResult(emptyList(), isCorrupt = true)
        }
        if (loadResult.isCorrupt) {
            metadataPersistenceFailed = true
            return emptyMap()
        }
        var correlations: Map<String, CorrelatedPurchase<Context>> = emptyMap()
        loadResult.metadata.forEach { metadata ->
            if (correlations.size >= MAX_CORRELATED_PURCHASES) return@forEach
            if (metadata.purchaseToken.isBlank() || metadata.productIds.any(String::isBlank) ||
                metadata.productIds.isEmpty()
            ) {
                return@forEach
            }
            val existing = correlations[metadata.purchaseToken]
            correlations = correlations + (
                metadata.purchaseToken to if (existing == null) {
                    CorrelatedPurchase(
                        purchaseToken = metadata.purchaseToken,
                        productIds = metadata.productIds,
                        context = metadata.context,
                        isDurable = true,
                    )
                } else {
                    CorrelatedPurchase(
                        purchaseToken = metadata.purchaseToken,
                        productIds = existing.productIds + metadata.productIds,
                        context = null,
                        isDurable = true,
                    )
                }
            )
        }
        return correlations
    }

    private fun durableCorrelations(): Map<String, CorrelatedPurchase<Context>> =
        (deferredPurchases + completedCorrelations)
            .filterValues(CorrelatedPurchase<Context>::isDurable)

    private fun persistDurableCorrelations(
        deferred: Map<String, CorrelatedPurchase<Context>> = deferredPurchases,
        completed: Map<String, CorrelatedPurchase<Context>> = completedCorrelations,
        force: Boolean = false,
    ): Boolean {
        val metadata = (deferred + completed).values
            .filter(CorrelatedPurchase<Context>::isDurable)
            .take(MAX_CORRELATED_PURCHASES)
            .map { correlation ->
                LocalTransactionMetadata(
                    purchaseToken = correlation.purchaseToken,
                    productIds = correlation.productIds,
                    context = correlation.context,
                )
            }
        if (metadata.isEmpty() && !force) return !metadataPersistenceFailed
        val didPersist = runCatching { metadataStore.replace(metadata) }.getOrDefault(false)
        metadataPersistenceFailed = !didPersist
        return didPersist
    }
}
