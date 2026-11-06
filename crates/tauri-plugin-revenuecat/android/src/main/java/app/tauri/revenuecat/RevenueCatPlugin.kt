package app.tauri.revenuecat

import android.app.Activity
import android.os.Handler
import android.os.Looper
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.android.billingclient.api.AcknowledgePurchaseParams
import com.android.billingclient.api.BillingClient
import com.android.billingclient.api.BillingClientStateListener
import com.android.billingclient.api.BillingFlowParams
import com.android.billingclient.api.BillingResult
import com.android.billingclient.api.ConsumeParams
import com.android.billingclient.api.PendingPurchasesParams
import com.android.billingclient.api.ProductDetails
import com.android.billingclient.api.Purchase
import com.android.billingclient.api.PurchasesUpdatedListener
import com.android.billingclient.api.QueryProductDetailsParams
import com.android.billingclient.api.QueryPurchasesParams
import org.json.JSONObject
import java.util.concurrent.ConcurrentLinkedQueue
import kotlin.math.min

@InvokeArg
internal class QueryProductsArgs {
    var ids: List<String> = listOf()
    var queries: List<ProductQueryArgs> = listOf()
}

@InvokeArg
internal class ProductQueryArgs {
    lateinit var productId: String
    var basePlanId: String? = null
}

@InvokeArg
internal class PurchaseArgs {
    lateinit var productId: String
    var appUserId: String? = null
    var basePlanId: String? = null
    var offerId: String? = null
    var offerToken: String? = null
    var receiptContext: NativeReceiptContextArgs? = null
}

@InvokeArg
internal class FinishTransactionArgs {
    lateinit var purchaseToken: String
    var shouldConsume: Boolean = false
}

@TauriPlugin
class RevenueCatPlugin(private val activity: Activity) :
    Plugin(activity), PurchasesUpdatedListener, BillingClientStateListener {

    private companion object {
        const val RECONNECT_TIMER_START_MILLISECONDS = 1L * 1000L
        const val RECONNECT_TIMER_MAX_TIME_MILLISECONDS = 1000L * 60L * 15L
        const val ERROR_CANCELLED = "cancelled"
        const val ERROR_PAYMENT_PENDING = "payment pending"
        const val ERROR_METADATA_PERSISTENCE =
            "Failed to persist Play transaction metadata for recovery"
        const val EVENT_TRANSACTION_UPDATED = "transaction-updated"
    }

    private val mainHandler = Handler(Looper.getMainLooper())

    @Volatile
    private var billingClient: BillingClient? = null

    private val serviceRequests = ConcurrentLinkedQueue<(connectionError: String?) -> Unit>()
    private var reconnectMilliseconds = RECONNECT_TIMER_START_MILLISECONDS
    private var reconnectionAlreadyScheduled = false

    private val pendingPurchases = PendingPurchaseStore<Invoke, NativePurchaseContext>(
        localTransactionMetadataStore(activity),
    )
    private var productDetailsCache: Map<String, ProductDetails> = emptyMap()

    // region Commands

    @Command
    fun isSandboxEnvironment(invoke: Invoke) {
        invoke.resolve(unknownSandboxEnvironmentResponse())
    }

    @Command
    fun queryProducts(invoke: Invoke) {
        val args = invoke.parseArgs(QueryProductsArgs::class.java)
        val queries = args.toProductQueries()
        if (queries.isEmpty()) {
            invoke.resolve(productsResponse(emptyList(), emptyList()))
            return
        }
        val ids = queries.map(ProductQuery::productId).toSet()
        executeRequestOnUIThread { connectionError ->
            if (connectionError != null) {
                invoke.reject(connectionError)
                return@executeRequestOnUIThread
            }
            // Play only accepts one product type per query: SUBS first, then INAPP for the rest.
            queryProductDetails(ids, BillingClient.ProductType.SUBS, invoke::reject) { subs ->
                val remaining = ids - subs.map { it.productId }.toSet()
                if (remaining.isEmpty()) {
                    invoke.resolve(productsResponse(queries, subs))
                } else {
                    queryProductDetails(remaining, BillingClient.ProductType.INAPP, invoke::reject) { inApp ->
                        invoke.resolve(productsResponse(queries, subs + inApp))
                    }
                }
            }
        }
    }

    @Command
    fun purchase(invoke: Invoke) {
        val args = invoke.parseArgs(PurchaseArgs::class.java)
        // launchBillingFlow requires a live foreground Activity to host the Play purchase sheet.
        if (activity.isFinishing || activity.isDestroyed) {
            invoke.reject("Activity is not available to launch the billing flow")
            return
        }
        executeRequestOnUIThread { connectionError ->
            if (connectionError != null) {
                invoke.reject(connectionError)
                return@executeRequestOnUIThread
            }
            withProductDetails(args.productId, invoke::reject) { details ->
                // This complete same-type snapshot is the boundary that prevents a stale Play
                // token from consuming a newer callback. Deliberately fail closed on query errors.
                queryOwnedPurchases(details.productType, invoke::reject) { ownedPurchases ->
                    val snapshots = ownedPurchases.mapNotNull { it.toOwnedPurchaseSnapshot() }
                    when (pendingPurchases.prepareForPurchase(args.productId, snapshots)) {
                        PurchasePreparation.READY -> beginPurchase(invoke, args, details, snapshots)
                        PurchasePreparation.PAYMENT_PENDING -> invoke.reject(ERROR_PAYMENT_PENDING)
                        PurchasePreparation.CORRELATION_CAPACITY_EXCEEDED -> invoke.reject(
                            "Too many unprocessed Play purchases; query or restore purchases first",
                        )
                        PurchasePreparation.METADATA_PERSISTENCE_FAILED -> invoke.reject(
                            ERROR_METADATA_PERSISTENCE,
                        )
                    }
                }
            }
        }
    }

    @Command
    fun queryPurchases(invoke: Invoke) {
        executePurchaseQuery(invoke) { purchase ->
            purchase.purchaseState == Purchase.PurchaseState.PURCHASED
        }
    }

    @Command
    fun queryPendingPurchases(invoke: Invoke) {
        executePurchaseQuery(invoke) { purchase ->
            shouldSyncPendingPurchase(
                state = purchase.toOwnedPurchaseSnapshot()?.state,
                isAcknowledged = purchase.isAcknowledged,
            )
        }
    }

    private fun executePurchaseQuery(
        invoke: Invoke,
        shouldInclude: (Purchase) -> Boolean,
    ) {
        executeRequestOnUIThread { connectionError ->
            if (connectionError != null) {
                invoke.reject(connectionError)
                return@executeRequestOnUIThread
            }
            queryAllOwnedPurchases(invoke::reject) { ownedPurchases ->
                val purchases = JSArray()
                val deferredContexts = pendingPurchases.reconcileOwnedPurchases(
                    ownedPurchases.mapNotNull { it.toOwnedPurchaseSnapshot() },
                )
                if (pendingPurchases.hasMetadataPersistenceFailure()) {
                    invoke.reject(ERROR_METADATA_PERSISTENCE)
                    return@queryAllOwnedPurchases
                }
                val completedPurchases = ownedPurchases.filter(shouldInclude)
                completedPurchases.forEach { purchase ->
                    purchases.put(
                        purchaseToJson(
                            purchase,
                            deferredContexts[purchase.purchaseToken],
                        ),
                    )
                }
                val result = JSObject()
                result.put("purchases", purchases)
                invoke.resolve(result)
            }
        }
    }

    @Command
    fun finishTransaction(invoke: Invoke) {
        val args = invoke.parseArgs(FinishTransactionArgs::class.java)
        executeRequestOnUIThread { connectionError ->
            if (connectionError != null) {
                invoke.reject(connectionError)
                return@executeRequestOnUIThread
            }
            // Rust reaches this command only after its backend accepted the receipt. Commit the
            // exact-token journal deletion before touching Play so a process death after
            // acknowledge/consume cannot leave an orphaned metadata record.
            if (!pendingPurchases.removeCorrelationBeforeStoreFinish(args.purchaseToken)) {
                invoke.reject("Failed to clear local metadata; Play transaction was not finished")
                return@executeRequestOnUIThread
            }
            if (args.shouldConsume) {
                withConnectedClient(invoke::reject) {
                    val params = ConsumeParams.newBuilder()
                        .setPurchaseToken(args.purchaseToken)
                        .build()
                    consumeAsync(params) { billingResult, _ ->
                        handleConsumeTransactionResult(invoke, billingResult)
                    }
                }
            } else {
                acknowledgeIfNeeded(invoke, args.purchaseToken)
            }
        }
    }

    // Play refunds any purchase not acknowledged within 3 days, so every
    // non-consumed purchase (subs and entitlement in-apps) must be
    // acknowledged — but acknowledging twice is a developer error, and
    // restores can hand us already-acknowledged purchases. Look the token up
    // and skip the call when it is already done.
    private fun acknowledgeIfNeeded(invoke: Invoke, purchaseToken: String) {
        queryOwnedPurchases(BillingClient.ProductType.SUBS, invoke::reject) { subs ->
            queryOwnedPurchases(BillingClient.ProductType.INAPP, invoke::reject) { inApp ->
                val owned = (subs + inApp).firstOrNull { it.purchaseToken == purchaseToken }
                if (!shouldCallPlayAcknowledge(owned != null, owned?.isAcknowledged == true)) {
                    invoke.resolve(JSObject())
                } else {
                    withConnectedClient(invoke::reject) {
                        val params = AcknowledgePurchaseParams.newBuilder()
                            .setPurchaseToken(purchaseToken)
                            .build()
                        acknowledgePurchase(params) { billingResult ->
                            handleFinishTransactionResult(invoke, billingResult)
                        }
                    }
                }
            }
        }
    }

    // endregion

    // region Connection management

    @Synchronized
    private fun executeRequestOnUIThread(request: (connectionError: String?) -> Unit) {
        serviceRequests.add(request)
        if (billingClient?.isReady == true) {
            executePendingRequests()
        } else {
            startConnection()
        }
    }

    private fun executePendingRequests() {
        synchronized(this) {
            while (billingClient?.isReady == true) {
                val request = serviceRequests.poll() ?: break
                mainHandler.post { request(null) }
            }
        }
    }

    private fun startConnection(delayMilliseconds: Long = 0L) {
        mainHandler.postDelayed({
            val clientToStart = synchronized(this) {
                if (billingClient == null) {
                    billingClient = BillingClient.newBuilder(activity)
                        .enablePendingPurchases(
                            PendingPurchasesParams.newBuilder()
                                .enableOneTimeProducts()
                                .enablePrepaidPlans()
                                .build(),
                        )
                        .setListener(this)
                        .enableAutoServiceReconnection()
                        .build()
                }
                reconnectionAlreadyScheduled = false
                billingClient?.takeIf { !it.isReady }
            } ?: return@postDelayed
            try {
                clientToStart.startConnection(this)
            } catch (e: IllegalStateException) {
                sendErrorsToAllPendingRequests("Play Store service error: ${e.message}")
            }
        }, delayMilliseconds)
    }

    override fun onBillingSetupFinished(billingResult: BillingResult) {
        mainHandler.post {
            when (billingResult.responseCode) {
                BillingClient.BillingResponseCode.OK -> {
                    reconnectMilliseconds = RECONNECT_TIMER_START_MILLISECONDS
                    executePendingRequests()
                }

                BillingClient.BillingResponseCode.FEATURE_NOT_SUPPORTED,
                BillingClient.BillingResponseCode.BILLING_UNAVAILABLE,
                -> sendErrorsToAllPendingRequests(
                    "Billing is not available on this device: ${billingResult.debugMessage}",
                )

                BillingClient.BillingResponseCode.ERROR,
                BillingClient.BillingResponseCode.SERVICE_UNAVAILABLE,
                BillingClient.BillingResponseCode.USER_CANCELED,
                BillingClient.BillingResponseCode.SERVICE_DISCONNECTED,
                BillingClient.BillingResponseCode.NETWORK_ERROR,
                -> retryBillingServiceConnectionWithExponentialBackoff()

                else -> sendErrorsToAllPendingRequests(
                    "Play Store setup failed (code ${billingResult.responseCode}): " +
                        billingResult.debugMessage,
                )
            }
        }
    }

    override fun onBillingServiceDisconnected() {
        // The next queued request triggers startConnection; no eager reconnect needed here.
    }

    // Exponential backoff instead of immediate retries: prevents ANRs when the Play Store
    // service is flapping (see android/play-billing-samples#310).
    @Synchronized
    private fun retryBillingServiceConnectionWithExponentialBackoff() {
        if (reconnectionAlreadyScheduled) return
        reconnectionAlreadyScheduled = true
        startConnection(reconnectMilliseconds)
        reconnectMilliseconds = min(reconnectMilliseconds * 2, RECONNECT_TIMER_MAX_TIME_MILLISECONDS)
    }

    @Synchronized
    private fun sendErrorsToAllPendingRequests(error: String) {
        while (true) {
            val request = serviceRequests.poll() ?: break
            mainHandler.post { request(error) }
        }
    }

    private fun withConnectedClient(
        onDisconnected: (error: String) -> Unit,
        receivingFunction: BillingClient.() -> Unit,
    ) {
        billingClient?.takeIf { it.isReady }?.receivingFunction()
            ?: onDisconnected("Billing client disconnected before the request could run")
    }

    // endregion

    // region Purchase flow

    private fun beginPurchase(
        invoke: Invoke,
        args: PurchaseArgs,
        details: ProductDetails,
        ownedPurchases: List<OwnedPurchaseSnapshot>,
    ) {
        val productId = args.productId
        if (!pendingPurchases.register(productId, invoke)) {
            invoke.reject("Another purchase is already in progress")
            return
        }
        val selectedOffer = selectSubscriptionOffer(details, args.toSubscriptionOptionRequest())
        if (details.productType == BillingClient.ProductType.SUBS && selectedOffer == null) {
            failPendingPurchase(productId, requestedSubscriptionOptionNotFoundMessage(args))
            return
        }
        val receiptContext = args.receiptContext?.toValidatedReceiptContext()
        if (args.receiptContext != null && receiptContext == null) {
            failPendingPurchase(productId, "Native receipt attribution context is invalid")
            return
        }
        val selectedOption = selectedOffer?.toDescriptor()
        if (selectedOption != null || receiptContext != null) {
            pendingPurchases.updateContext(
                productId,
                NativePurchaseContext(
                    productId = productId,
                    basePlanId = selectedOption?.basePlanId,
                    offerId = selectedOption?.offerId,
                    receiptContext = receiptContext,
                ),
            )
        }
        val obfuscatedAccountId = obfuscatedAccountIdForPurchase(
            appUserId = args.appUserId,
            productId = productId,
            isSubscription = details.productType == BillingClient.ProductType.SUBS,
            ownedPurchases = ownedPurchases,
        )
        launchBillingFlow(productId, details, selectedOffer, obfuscatedAccountId)
    }

    private fun launchBillingFlow(
        productId: String,
        details: ProductDetails,
        selectedOffer: ProductDetails.SubscriptionOfferDetails?,
        obfuscatedAccountId: String?,
    ) {
        val productDetailsParams = BillingFlowParams.ProductDetailsParams.newBuilder()
            .setProductDetails(details)
            .apply { selectedOffer?.let { setOfferToken(it.offerToken) } }
            .build()
        val flowParams = BillingFlowParams.newBuilder()
            .setProductDetailsParamsList(listOf(productDetailsParams))
            .apply { obfuscatedAccountId?.let { setObfuscatedAccountId(it) } }
            .build()
        withConnectedClient(onDisconnected = { failPendingPurchase(productId, it) }) {
            pendingPurchases.markLaunched(productId, System.currentTimeMillis())
            val billingResult = launchBillingFlow(activity, flowParams)
            if (billingResult.responseCode != BillingClient.BillingResponseCode.OK) {
                failPendingPurchase(
                    productId,
                    "Failed to launch billing flow (code ${billingResult.responseCode}): " +
                        billingResult.debugMessage,
                )
            }
        }
    }

    override fun onPurchasesUpdated(billingResult: BillingResult, purchases: List<Purchase>?) {
        val responseCode = billingResult.responseCode
        if (responseCode == BillingClient.BillingResponseCode.OK && !purchases.isNullOrEmpty()) {
            purchases.forEach { handleUpdatedPurchase(it) }
            return
        }
        val message = when {
            responseCode == BillingClient.BillingResponseCode.USER_CANCELED -> ERROR_CANCELLED
            responseCode == BillingClient.BillingResponseCode.OK ->
                "Purchase update returned OK without any purchases"
            else -> "Purchase failed (code $responseCode): ${billingResult.debugMessage}"
        }
        drainPendingPurchaseInvokes().forEach { it.reject(message) }
    }

    private fun handleUpdatedPurchase(purchase: Purchase) {
        when (purchase.purchaseState) {
            Purchase.PurchaseState.PURCHASED -> resolvePendingPurchase(purchase)
            Purchase.PurchaseState.PENDING -> rejectPendingPurchase(purchase)
            else -> pendingPurchases
                .takeForProducts(purchase.products)
                .forEach { it.callback.reject("Purchase returned an unspecified state") }
        }
    }

    private fun resolvePendingPurchase(purchase: Purchase) {
        val route = pendingPurchases.routePurchased(
            purchase.purchaseToken,
            purchase.products,
            purchase.purchaseTime,
        )
        when (route.kind) {
            PurchaseUpdateKind.ACTIVE -> route.activePurchase?.let { pending ->
                if (pending.isMetadataDurable) {
                    pending.callback.resolve(purchaseToJson(purchase, pending.context))
                } else {
                    pending.callback.reject(ERROR_METADATA_PERSISTENCE)
                    trigger(EVENT_TRANSACTION_UPDATED, JSObject())
                }
            }

            PurchaseUpdateKind.CORRELATED,
            PurchaseUpdateKind.OUT_OF_BAND,
            -> trigger(EVENT_TRANSACTION_UPDATED, JSObject())
        }
    }

    // Match the official SDK: a PENDING purchase completes the current API call with
    // PaymentPendingError. Retain only immutable correlation data (never the Invoke), so a later
    // ownership query can preserve a proven base-plan/offer identity without allowing the old
    // purchase to resolve a new callback.
    private fun rejectPendingPurchase(purchase: Purchase) {
        pendingPurchases
            .defer(purchase.products, purchase.purchaseToken)
            ?.let { pending ->
                val error = if (pending.isMetadataDurable) {
                    ERROR_PAYMENT_PENDING
                } else {
                    ERROR_METADATA_PERSISTENCE
                }
                pending.callback.reject(error)
            }
    }

    private fun failPendingPurchase(productId: String, message: String) {
        val pending = pendingPurchases.remove(productId) ?: return
        pending.callback.reject(message)
    }

    private fun drainPendingPurchaseInvokes(): List<Invoke> = pendingPurchases
        .drain()
        .map(PendingPurchase<Invoke, NativePurchaseContext>::callback)

    // endregion

    // region Queries

    private fun queryProductDetails(
        ids: Set<String>,
        productType: String,
        onError: (error: String) -> Unit,
        onSuccess: (products: List<ProductDetails>) -> Unit,
    ) {
        val productList = ids.map { id ->
            QueryProductDetailsParams.Product.newBuilder()
                .setProductId(id)
                .setProductType(productType)
                .build()
        }
        val params = QueryProductDetailsParams.newBuilder()
            .setProductList(productList)
            .build()
        withConnectedClient(onError) {
            queryProductDetailsAsync(params) { billingResult, result ->
                if (billingResult.responseCode == BillingClient.BillingResponseCode.OK) {
                    synchronized(this@RevenueCatPlugin) {
                        productDetailsCache = productDetailsCache +
                            result.productDetailsList.associateBy(ProductDetails::getProductId)
                    }
                    onSuccess(result.productDetailsList)
                } else {
                    onError(
                        "Failed to query products (code ${billingResult.responseCode}): " +
                            billingResult.debugMessage,
                    )
                }
            }
        }
    }

    private fun withProductDetails(
        productId: String,
        onError: (error: String) -> Unit,
        onSuccess: (details: ProductDetails) -> Unit,
    ) {
        synchronized(this) { productDetailsCache[productId] }?.let {
            onSuccess(it)
            return
        }
        queryProductDetails(setOf(productId), BillingClient.ProductType.SUBS, onError) { subs ->
            val subscription = subs.firstOrNull()
            if (subscription != null) {
                onSuccess(subscription)
            } else {
                queryProductDetails(setOf(productId), BillingClient.ProductType.INAPP, onError) { inApp ->
                    inApp.firstOrNull()?.let(onSuccess)
                        ?: onError("Product not found on Play: $productId")
                }
            }
        }
    }

    private fun queryOwnedPurchases(
        productType: String,
        onError: (error: String) -> Unit,
        onSuccess: (purchases: List<Purchase>) -> Unit,
    ) {
        val params = QueryPurchasesParams.newBuilder()
            .setProductType(productType)
            .build()
        withConnectedClient(onError) {
            queryPurchasesAsync(params) { billingResult, purchases ->
                if (billingResult.responseCode == BillingClient.BillingResponseCode.OK) {
                    onSuccess(purchases)
                } else {
                    onError(
                        "Failed to query purchases (code ${billingResult.responseCode}): " +
                            billingResult.debugMessage,
                    )
                }
            }
        }
    }

    private fun queryAllOwnedPurchases(
        onError: (error: String) -> Unit,
        onSuccess: (purchases: List<Purchase>) -> Unit,
    ) {
        queryOwnedPurchases(BillingClient.ProductType.SUBS, onError) { subscriptions ->
            queryOwnedPurchases(BillingClient.ProductType.INAPP, onError) { inApp ->
                onSuccess(subscriptions + inApp)
            }
        }
    }

    private fun handleFinishTransactionResult(
        invoke: Invoke,
        billingResult: BillingResult,
    ) {
        if (isIdempotentFinishSuccess(billingResult.responseCode)) {
            invoke.resolve(JSObject())
        } else {
            invoke.reject(
                "Failed to finish transaction (code ${billingResult.responseCode}): " +
                    billingResult.debugMessage,
            )
        }
    }

    private fun handleConsumeTransactionResult(
        invoke: Invoke,
        billingResult: BillingResult,
    ) {
        if (isIdempotentFinishSuccess(billingResult.responseCode)) {
            invoke.resolve(JSObject())
        } else {
            invoke.reject(
                "Failed to finish transaction (code ${billingResult.responseCode}): " +
                    billingResult.debugMessage,
            )
        }
    }

    // endregion

    // region Serialization

    private fun productsResponse(
        queries: List<ProductQuery>,
        products: List<ProductDetails>,
    ): JSObject {
        val productsById = products.associateBy(ProductDetails::getProductId)
        val array = JSArray()
        queries.forEach { query ->
            productsById[query.productId]
                ?.let { productDetailsToJson(it, query.basePlanId) }
                ?.let(array::put)
        }
        return JSObject().apply { put("products", array) }
    }

    private fun productDetailsToJson(
        details: ProductDetails,
        requestedBasePlanId: String? = null,
    ): JSObject? {
        val isSubscription = details.productType == BillingClient.ProductType.SUBS
        val selectedOffer = selectSubscriptionOffer(
            details,
            SubscriptionOptionRequest(basePlanId = requestedBasePlanId),
        )
        if (isSubscription && selectedOffer == null) return null
        val selectedOption = selectedOffer?.toDescriptor()
        // The last pricing phase of the base plan is the recurring base price;
        // earlier phases are free trials or intro offers.
        val basePricingPhase = selectedOffer?.pricingPhases?.pricingPhaseList?.lastOrNull()
        val oneTimeOffer = details.oneTimePurchaseOfferDetails
        return JSObject().apply {
            put("identifier", details.productId)
            // Play ProductDetails does not identify whether an INAPP product is
            // consumable. Default to non-consumable; Rust uses RevenueCat's
            // `purchased_products.should_consume` receipt response as authority.
            put("productType", if (isSubscription) "subscription" else "non_consumable")
            put("title", details.title)
            put("description", details.description)
            put("priceMicros", basePricingPhase?.priceAmountMicros ?: oneTimeOffer?.priceAmountMicros ?: 0L)
            put("currency", basePricingPhase?.priceCurrencyCode ?: oneTimeOffer?.priceCurrencyCode ?: "")
            put("period", basePricingPhase?.billingPeriod ?: JSONObject.NULL)
            selectedOption?.let { option ->
                put("basePlanId", option.basePlanId)
                option.offerId?.let { put("offerId", it) }
                put("offerToken", option.offerToken)
            }
        }
    }

    private fun selectSubscriptionOffer(
        details: ProductDetails,
        request: SubscriptionOptionRequest,
    ): ProductDetails.SubscriptionOfferDetails? {
        val offers = details.subscriptionOfferDetails.orEmpty()
        if (offers.isEmpty()) return null
        val descriptors = offers.map { it.toDescriptor() }
        val selected = selectSubscriptionOption(descriptors, request) ?: return null
        return offers.firstOrNull { it.offerToken == selected.offerToken }
    }

    private fun purchaseToJson(
        purchase: Purchase,
        purchaseContext: NativePurchaseContext? = null,
    ): JSObject {
        val productIds = JSArray()
        purchase.products.forEach { productIds.put(it) }
        val platformProductIds = JSArray()
        purchase.products.forEach { productId ->
            platformProductIds.put(
                JSObject().apply {
                    put("productId", productId)
                    purchaseContext
                        ?.takeIf { it.productId == productId }
                        ?.basePlanId
                        ?.let { basePlanId ->
                            put("basePlanId", basePlanId)
                            purchaseContext.offerId?.let { put("offerId", it) }
                        }
                },
            )
        }
        return JSObject().apply {
            put("purchaseToken", purchase.purchaseToken)
            put("productIds", productIds)
            put("platformProductIds", platformProductIds)
            put("transactionId", purchase.orderId ?: JSONObject.NULL)
            put("purchaseTimeMs", purchase.purchaseTime)
            purchaseContext?.receiptContext?.let {
                put("receiptContext", it.toJsonObject())
            }
        }
    }

    private fun ProductDetails.SubscriptionOfferDetails.toDescriptor() = SubscriptionOptionDescriptor(
        basePlanId = basePlanId,
        offerId = offerId,
        offerToken = offerToken,
        tags = offerTags.toSet(),
        pricingPhases = pricingPhases.pricingPhaseList.map { phase ->
            SubscriptionPricingPhase(
                priceAmountMicros = phase.priceAmountMicros,
                billingPeriod = phase.billingPeriod,
            )
        },
    )

    private fun PurchaseArgs.toSubscriptionOptionRequest() = SubscriptionOptionRequest(
        basePlanId = basePlanId,
        offerId = offerId,
        offerToken = offerToken,
    )

    private fun requestedSubscriptionOptionNotFoundMessage(args: PurchaseArgs): String =
        "Subscription option is unavailable for ${args.productId}" +
            (args.basePlanId?.let { " (base plan $it)" } ?: "") +
            (args.offerId?.let { " (offer $it)" } ?: "")

    private fun QueryProductsArgs.toProductQueries(): List<ProductQuery> = normalizeProductQueries(
        ids = ids,
        queries = queries.map { ProductQuery(it.productId, it.basePlanId) },
    )

    private fun Purchase.toOwnedPurchaseSnapshot(): OwnedPurchaseSnapshot? {
        val state = when (purchaseState) {
            Purchase.PurchaseState.PENDING -> OwnedPurchaseState.PENDING
            Purchase.PurchaseState.PURCHASED -> OwnedPurchaseState.PURCHASED
            else -> return null
        }
        return OwnedPurchaseSnapshot(
            purchaseToken = purchaseToken,
            productIds = products.toSet(),
            state = state,
        )
    }

    // endregion
}

internal fun isIdempotentFinishSuccess(responseCode: Int): Boolean =
    responseCode == BillingClient.BillingResponseCode.OK ||
        responseCode == BillingClient.BillingResponseCode.ITEM_NOT_OWNED

internal fun shouldCallPlayAcknowledge(isOwned: Boolean, isAcknowledged: Boolean): Boolean =
    isOwned && !isAcknowledged
