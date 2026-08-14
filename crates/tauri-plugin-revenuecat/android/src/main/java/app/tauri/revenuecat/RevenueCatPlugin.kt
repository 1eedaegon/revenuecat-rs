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
}

@InvokeArg
internal class PurchaseArgs {
    lateinit var productId: String
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
    }

    private val mainHandler = Handler(Looper.getMainLooper())

    @Volatile
    private var billingClient: BillingClient? = null

    private val serviceRequests = ConcurrentLinkedQueue<(connectionError: String?) -> Unit>()
    private var reconnectMilliseconds = RECONNECT_TIMER_START_MILLISECONDS
    private var reconnectionAlreadyScheduled = false

    private val pendingPurchaseInvokes = mutableMapOf<String, Invoke>()
    private val productDetailsCache = mutableMapOf<String, ProductDetails>()

    // region Commands

    @Command
    fun queryProducts(invoke: Invoke) {
        val ids = invoke.parseArgs(QueryProductsArgs::class.java).ids.toSet()
        if (ids.isEmpty()) {
            invoke.resolve(productsResponse(emptyList()))
            return
        }
        executeRequestOnUIThread { connectionError ->
            if (connectionError != null) {
                invoke.reject(connectionError)
                return@executeRequestOnUIThread
            }
            // Play only accepts one product type per query: SUBS first, then INAPP for the rest.
            queryProductDetails(ids, BillingClient.ProductType.SUBS, invoke::reject) { subs ->
                val remaining = ids - subs.map { it.productId }.toSet()
                if (remaining.isEmpty()) {
                    invoke.resolve(productsResponse(subs))
                } else {
                    queryProductDetails(remaining, BillingClient.ProductType.INAPP, invoke::reject) { inApp ->
                        invoke.resolve(productsResponse(subs + inApp))
                    }
                }
            }
        }
    }

    @Command
    fun purchase(invoke: Invoke) {
        val productId = invoke.parseArgs(PurchaseArgs::class.java).productId
        // launchBillingFlow requires a live foreground Activity to host the Play purchase sheet.
        if (activity.isFinishing || activity.isDestroyed) {
            invoke.reject("Activity is not available to launch the billing flow")
            return
        }
        synchronized(this) {
            if (pendingPurchaseInvokes.isNotEmpty()) {
                invoke.reject("Another purchase is already in progress")
                return
            }
            pendingPurchaseInvokes[productId] = invoke
        }
        executeRequestOnUIThread { connectionError ->
            if (connectionError != null) {
                failPendingPurchase(productId, connectionError)
                return@executeRequestOnUIThread
            }
            withProductDetails(productId, onError = { failPendingPurchase(productId, it) }) { details ->
                launchBillingFlow(productId, details)
            }
        }
    }

    @Command
    fun queryPurchases(invoke: Invoke) {
        executeRequestOnUIThread { connectionError ->
            if (connectionError != null) {
                invoke.reject(connectionError)
                return@executeRequestOnUIThread
            }
            queryOwnedPurchases(BillingClient.ProductType.SUBS, invoke::reject) { subs ->
                queryOwnedPurchases(BillingClient.ProductType.INAPP, invoke::reject) { inApp ->
                    val purchases = JSArray()
                    (subs + inApp).forEach { purchases.put(purchaseToJson(it)) }
                    val result = JSObject()
                    result.put("purchases", purchases)
                    invoke.resolve(result)
                }
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
            if (args.shouldConsume) {
                withConnectedClient(invoke::reject) {
                    val params = ConsumeParams.newBuilder()
                        .setPurchaseToken(args.purchaseToken)
                        .build()
                    consumeAsync(params) { billingResult, _ ->
                        handleFinishTransactionResult(invoke, billingResult)
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
                if (owned != null && owned.isAcknowledged) {
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
                            PendingPurchasesParams.newBuilder().enableOneTimeProducts().build(),
                        )
                        .setListener(this)
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

                else -> Unit
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

    private fun launchBillingFlow(productId: String, details: ProductDetails) {
        val productDetailsParams = BillingFlowParams.ProductDetailsParams.newBuilder()
            .setProductDetails(details)
            .apply { baseOffer(details)?.let { setOfferToken(it.offerToken) } }
            .build()
        val flowParams = BillingFlowParams.newBuilder()
            .setProductDetailsParamsList(listOf(productDetailsParams))
            .build()
        withConnectedClient(onDisconnected = { failPendingPurchase(productId, it) }) {
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
        val invokes = synchronized(this) {
            purchase.products.mapNotNull { pendingPurchaseInvokes.remove(it) }
        }
        invokes.forEach { invoke ->
            if (purchase.purchaseState == Purchase.PurchaseState.PURCHASED) {
                invoke.resolve(purchaseToJson(purchase))
            } else {
                // PENDING purchases have no chargeable token yet; they complete outside this flow
                // and will show up in queryPurchases once Play finishes them.
                invoke.reject("Purchase is pending")
            }
        }
    }

    private fun failPendingPurchase(productId: String, message: String) {
        val invoke = synchronized(this) { pendingPurchaseInvokes.remove(productId) } ?: return
        invoke.reject(message)
    }

    private fun drainPendingPurchaseInvokes(): List<Invoke> = synchronized(this) {
        val invokes = pendingPurchaseInvokes.values.toList()
        pendingPurchaseInvokes.clear()
        invokes
    }

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
                        result.productDetailsList.forEach { productDetailsCache[it.productId] = it }
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

    private fun handleFinishTransactionResult(invoke: Invoke, billingResult: BillingResult) {
        if (billingResult.responseCode == BillingClient.BillingResponseCode.OK) {
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

    private fun productsResponse(products: List<ProductDetails>): JSObject {
        val array = JSArray()
        products.forEach { array.put(productDetailsToJson(it)) }
        val result = JSObject()
        result.put("products", array)
        return result
    }

    private fun productDetailsToJson(details: ProductDetails): JSObject {
        val isSubscription = details.productType == BillingClient.ProductType.SUBS
        // The last pricing phase of the base plan is the recurring base price;
        // earlier phases are free trials or intro offers.
        val basePricingPhase = baseOffer(details)?.pricingPhases?.pricingPhaseList?.lastOrNull()
        val oneTimeOffer = details.oneTimePurchaseOfferDetails
        return JSObject().apply {
            put("identifier", details.productId)
            put("productType", if (isSubscription) "subscription" else "consumable")
            put("title", details.title)
            put("description", details.description)
            put("priceMicros", basePricingPhase?.priceAmountMicros ?: oneTimeOffer?.priceAmountMicros ?: 0L)
            put("currency", basePricingPhase?.priceCurrencyCode ?: oneTimeOffer?.priceCurrencyCode ?: "")
            put("period", basePricingPhase?.billingPeriod ?: JSONObject.NULL)
        }
    }

    // Offers without an offerId are the bare base plans; prefer them for price and purchase.
    private fun baseOffer(
        details: ProductDetails,
    ): ProductDetails.SubscriptionOfferDetails? = details.subscriptionOfferDetails?.let { offers ->
        offers.firstOrNull { it.offerId == null } ?: offers.firstOrNull()
    }

    private fun purchaseToJson(purchase: Purchase): JSObject {
        val productIds = JSArray()
        purchase.products.forEach { productIds.put(it) }
        return JSObject().apply {
            put("purchaseToken", purchase.purchaseToken)
            put("productIds", productIds)
            put("transactionId", purchase.orderId ?: JSONObject.NULL)
            put("purchaseTimeMs", purchase.purchaseTime)
        }
    }

    // endregion
}
