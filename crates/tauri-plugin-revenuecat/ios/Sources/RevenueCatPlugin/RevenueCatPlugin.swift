import Foundation
import StoreKit
import Tauri
import WebKit

let nativeTransactionUpdatedEvent = "transaction-updated"

func appAccountToken(from appUserId: String?) -> UUID? {
    guard let appUserId else { return nil }
    return UUID(uuidString: appUserId)
}

func shouldUsePendingPurchaseContext(transactionReasonIsRenewal: Bool?) -> Bool {
    transactionReasonIsRenewal != true
}

// MARK: - Command arguments

private struct QueryProductsArgs: Decodable {
    let ids: [String]
}

private struct PurchaseArgs: Decodable {
    let productId: String
    let appUserId: String?
    let receiptContext: ReceiptContext?
}

private struct FinishTransactionArgs: Decodable {
    let purchaseToken: String
    // Optional: the Rust bridge sends null when the originating store
    // transaction carried no id (decoding a non-optional would throw and
    // fail the finish after an otherwise successful purchase).
    let transactionId: String?
    let shouldConsume: Bool
}

// MARK: - Response payloads

private struct ProductPayload: Encodable {
    let identifier: String
    let productType: String
    let title: String
    let description: String
    let priceMicros: Int64
    let currency: String
    let period: String?
}

private struct ProductsResponse: Encodable {
    let products: [ProductPayload]
}

struct PurchasePayload: Encodable, Equatable, Sendable {
    let purchaseToken: String
    let productIds: [String]
    let transactionId: String
    let purchaseTimeMs: Int64
    let receiptContext: ReceiptContext?

    init(
        purchaseToken: String,
        productIds: [String],
        transactionId: String,
        purchaseTimeMs: Int64,
        receiptContext: ReceiptContext? = nil
    ) {
        self.purchaseToken = purchaseToken
        self.productIds = productIds
        self.transactionId = transactionId
        self.purchaseTimeMs = purchaseTimeMs
        self.receiptContext = receiptContext
    }
}

struct PurchasePayloadAccumulator: Sendable {
    private let seenTransactionIds: Set<String>
    let payloads: [PurchasePayload]

    init() {
        seenTransactionIds = []
        payloads = []
    }

    func appending(_ payload: PurchasePayload) -> Self {
        guard !seenTransactionIds.contains(payload.transactionId) else {
            return self
        }
        return Self(
            seenTransactionIds: seenTransactionIds.union([payload.transactionId]),
            payloads: payloads + [payload]
        )
    }

    private init(
        seenTransactionIds: Set<String>,
        payloads: [PurchasePayload]
    ) {
        self.seenTransactionIds = seenTransactionIds
        self.payloads = payloads
    }
}

private struct PurchasesResponse: Encodable {
    let purchases: [PurchasePayload]
}

struct SandboxEnvironmentResponse: Encodable {
    let isSandbox: Bool?
}

// MARK: - Errors

private enum PurchaseFlowError: Error, CustomStringConvertible {
    case productNotFound(String)
    case cancelled
    case pending
    case alreadyInFlight
    case transactionNotFound(String)
    case transactionMismatch(String)
    case storeKit(Error)
    case localMetadata(Error)

    var description: String {
        switch self {
        case .productNotFound(let id):
            return "Product not found: \(id)"
        case .cancelled:
            return "cancelled"
        case .pending:
            return "payment pending"
        case .alreadyInFlight:
            return "A purchase is already in progress"
        case .transactionNotFound(let id):
            return "Unfinished transaction not found: \(id)"
        case .transactionMismatch(let id):
            return "Unfinished transaction payload did not match transaction: \(id)"
        case .storeKit(let error):
            return "StoreKit error: \(error.localizedDescription)"
        case .localMetadata(let error):
            return "Local transaction metadata error: \(error.localizedDescription)"
        }
    }
}

// MARK: - Purchase gate

@available(iOS 15.0, macOS 12.0, *)
actor PurchaseGate {
    private static let maximumRememberedTransactionIds = 32
    private var isInFlight = false
    private var deferredUpdateTransactionIds: Set<String> = []
    private var completedTransactionIdsToSuppress: [String] = []

    func begin() -> Bool {
        if isInFlight {
            return false
        }
        isInFlight = true
        return true
    }

    func shouldNotifyUpdate(transactionId: String) -> Bool {
        if let rememberedIndex = completedTransactionIdsToSuppress.firstIndex(of: transactionId) {
            completedTransactionIdsToSuppress.remove(at: rememberedIndex)
            return false
        }
        if isInFlight {
            deferredUpdateTransactionIds.insert(transactionId)
            return false
        }
        return true
    }

    /// Ends an explicit flow and returns whether a different transaction was
    /// deferred while StoreKit's purchase result was being handled.
    func end(explicitTransactionId: String?) -> Bool {
        isInFlight = false
        if let explicitTransactionId {
            if deferredUpdateTransactionIds.remove(explicitTransactionId) == nil {
                completedTransactionIdsToSuppress.append(explicitTransactionId)
                if completedTransactionIdsToSuppress.count > Self.maximumRememberedTransactionIds {
                    completedTransactionIdsToSuppress.removeFirst(
                        completedTransactionIdsToSuppress.count - Self.maximumRememberedTransactionIds
                    )
                }
            }
        }
        let shouldNotify = !deferredUpdateTransactionIds.isEmpty
        deferredUpdateTransactionIds.removeAll()
        return shouldNotify
    }
}

// MARK: - Plugin

@available(iOS 15.0, macOS 12.0, *)
class RevenueCatPlugin: Plugin {

    private let purchaseGate = PurchaseGate()
    private let localTransactionMetadata = LocalTransactionMetadataStore()
    private let pendingPurchaseContexts = PendingPurchaseContextCache()
    private var updatesTask: Task<Void, Never>?

    deinit {
        updatesTask?.cancel()
    }

    @objc public override func load(webview: WKWebView) {
        // `Transaction.updates` delivers renewals, Ask-to-Buy approvals, and purchases
        // made outside this app instance. Notify the guest binding, which asks Rust
        // to query/post every unfinished transaction. The native transaction remains
        // unfinished until the backend accepts it and Rust calls `finishTransaction`.
        updatesTask?.cancel()
        updatesTask = Task { [weak self] in
            for await result in Transaction.updates {
                guard !Task.isCancelled, case .verified(let transaction) = result else {
                    continue
                }
                do {
                    _ = try await self?.receiptContext(
                        for: transaction,
                        jwsRepresentation: result.jwsRepresentation
                    )
                } catch {
                    // Still notify Rust so it can retry queue reconciliation. The
                    // query path repeats this write and rejects while durable
                    // attribution cannot be established, preventing a contextless
                    // backend post followed by an irreversible StoreKit finish.
                    NSLog(
                        "RevenueCatPlugin could not persist transaction metadata: %@",
                        error.localizedDescription
                    )
                }
                guard await self?.purchaseGate.shouldNotifyUpdate(
                    transactionId: String(transaction.id)
                ) == true else { continue }
                await self?.notifyTransactionUpdated()
            }
        }
    }

    // MARK: isSandboxEnvironment

    @objc public func isSandboxEnvironment(_ invoke: Invoke) throws {
        invoke.resolve(
            SandboxEnvironmentResponse(
                isSandbox: currentSandboxEnvironment()
            )
        )
    }

    // MARK: queryProducts

    @objc public func queryProducts(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(QueryProductsArgs.self)
        Task {
            do {
                let products = try await Product.products(for: args.ids)
                // StoreKit returns products unordered; restore request order.
                let order = requestedProductOrder(args.ids)
                let payloads = products
                    .sorted { (order[$0.id] ?? .max) < (order[$1.id] ?? .max) }
                    .map(Self.productPayload(for:))
                invoke.resolve(ProductsResponse(products: payloads))
            } catch {
                invoke.reject(PurchaseFlowError.storeKit(error).description)
            }
        }
    }

    // MARK: purchase

    @objc public func purchase(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(PurchaseArgs.self)
        Task {
            guard await self.purchaseGate.begin() else {
                invoke.reject(PurchaseFlowError.alreadyInFlight.description)
                return
            }

            let outcome: Result<PurchasePayload, PurchaseFlowError>
            do {
                outcome = .success(
                    try await self.performPurchase(
                        productId: args.productId,
                        appUserId: args.appUserId,
                        receiptContext: args.receiptContext
                    )
                )
            } catch let error as PurchaseFlowError {
                outcome = .failure(error)
            } catch {
                outcome = .failure(.storeKit(error))
            }
            let explicitTransactionId: String?
            switch outcome {
            case .success(let payload): explicitTransactionId = payload.transactionId
            case .failure: explicitTransactionId = nil
            }
            let shouldNotifyDeferredUpdate = await self.purchaseGate.end(
                explicitTransactionId: explicitTransactionId
            )
            if shouldNotifyDeferredUpdate {
                await self.notifyTransactionUpdated()
            }

            switch outcome {
            case .success(let payload):
                invoke.resolve(payload)
            case .failure(let error):
                invoke.reject(error.description)
            }
        }
    }

    private func performPurchase(
        productId: String,
        appUserId: String?,
        receiptContext: ReceiptContext?
    ) async throws -> PurchasePayload {
        guard let product = try await Product.products(for: [productId]).first else {
            throw PurchaseFlowError.productNotFound(productId)
        }

        if let receiptContext {
            await pendingPurchaseContexts.cache(
                receiptContext,
                productId: productId
            )
        }

        let result: Product.PurchaseResult
        do {
            if let token = appAccountToken(from: appUserId) {
                result = try await product.purchase(options: [.appAccountToken(token)])
            } else {
                result = try await product.purchase()
            }
        } catch {
            await pendingPurchaseContexts.remove(productId: productId)
            throw PurchaseFlowError.storeKit(error)
        }

        switch result {
        case .success(let verification):
            switch verification {
            case .verified(let transaction):
                // The verified JWS stays unfinished until the backend
                // acknowledges it and `finishTransaction` matches it exactly.
                let payload = Self.purchasePayload(
                    for: transaction,
                    jwsRepresentation: verification.jwsRepresentation,
                    receiptContext: receiptContext
                )
                if let receiptContext {
                    do {
                        try await localTransactionMetadata.store(
                            LocalTransactionMetadata(
                                transactionId: payload.transactionId,
                                purchaseToken: payload.purchaseToken,
                                productId: productId,
                                receiptContext: receiptContext
                            )
                        )
                    } catch {
                        throw PurchaseFlowError.localMetadata(error)
                    }
                }
                await pendingPurchaseContexts.remove(productId: productId)
                return payload
            case .unverified(_, let verificationError):
                // Match purchases-ios: never pass an unverified StoreKit
                // transaction into a flow whose query/finish paths only
                // operate on verified transactions.
                await pendingPurchaseContexts.remove(productId: productId)
                throw PurchaseFlowError.storeKit(verificationError)
            }
        case .pending:
            // Ask to Buy / SCA: the transaction arrives later via Transaction.updates.
            throw PurchaseFlowError.pending
        case .userCancelled:
            await pendingPurchaseContexts.remove(productId: productId)
            throw PurchaseFlowError.cancelled
        @unknown default:
            await pendingPurchaseContexts.remove(productId: productId)
            throw PurchaseFlowError.storeKit(
                NSError(
                    domain: "RevenueCatPlugin",
                    code: -1,
                    userInfo: [NSLocalizedDescriptionKey: "Unknown purchase result"]
                )
            )
        }
    }

    // MARK: queryPurchases

    @objc public func queryPurchases(_ invoke: Invoke) throws {
        Task {
            do {
                var accumulator = PurchasePayloadAccumulator()
                // Prefer `unfinished` first: its JWS is the exact token that
                // `finishTransaction` will match after the backend accepts it.
                for payload in try await self.unfinishedPurchasePayloads() {
                    accumulator = accumulator.appending(payload)
                }
                // `all` includes finished consumables, expired subscriptions, and
                // inactive entitlements required by an explicit/full restore.
                // Duplicate transaction IDs retain the exact unfinished JWS above.
                for await result in Transaction.all {
                    guard case .verified(let transaction) = result else {
                        continue
                    }
                    accumulator = accumulator.appending(
                        Self.purchasePayload(
                            for: transaction,
                            jwsRepresentation: result.jwsRepresentation
                        )
                    )
                }

                invoke.resolve(PurchasesResponse(purchases: accumulator.payloads))
            } catch {
                invoke.reject(PurchaseFlowError.localMetadata(error).description)
            }
        }
    }

    /// Automatic queue reconciliation intentionally excludes finished history.
    /// Explicit/full restore uses `queryPurchases` above.
    @objc public func queryPendingPurchases(_ invoke: Invoke) throws {
        Task {
            do {
                invoke.resolve(
                    PurchasesResponse(purchases: try await self.unfinishedPurchasePayloads())
                )
            } catch {
                invoke.reject(PurchaseFlowError.localMetadata(error).description)
            }
        }
    }

    // MARK: finishTransaction

    @objc public func finishTransaction(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(FinishTransactionArgs.self)
        Task {
            guard let transactionId = args.transactionId else {
                // Without a StoreKit transaction id there is nothing to
                // match; the desired end state (nothing left unfinished for
                // this purchase) holds, so this is not an error.
                invoke.resolve()
                return
            }
            for await result in Transaction.unfinished {
                guard case .verified(let transaction) = result else {
                    continue
                }
                let actualTransactionId = String(transaction.id)
                guard actualTransactionId == transactionId else {
                    continue
                }
                guard finishPayloadMatches(
                    expectedTransactionId: transactionId,
                    expectedPurchaseToken: args.purchaseToken,
                    actualTransactionId: actualTransactionId,
                    actualPurchaseToken: result.jwsRepresentation
                ) else {
                    invoke.reject(PurchaseFlowError.transactionMismatch(transactionId).description)
                    return
                }
                // StoreKit 2 has no client-side consume; for consumables,
                // finish() is what removes the transaction from the queue.
                // `shouldConsume` is accepted for cross-platform contract parity.
                _ = args.shouldConsume
                do {
                    // Rust invokes this command only after RevenueCat accepts the
                    // receipt. Delete the exact journal entry first: a deletion
                    // failure leaves StoreKit unfinished for retry, while a crash
                    // after deletion can safely re-post the accepted transaction
                    // idempotently. Finishing first could orphan its JWS forever.
                    try await removeMetadataThenFinishTransaction(
                        removeMetadata: {
                            _ = try await self.localTransactionMetadata.remove(
                                transactionId: transactionId,
                                purchaseToken: args.purchaseToken
                            )
                        },
                        finishTransaction: {
                            await transaction.finish()
                        }
                    )
                    invoke.resolve()
                } catch {
                    invoke.reject(PurchaseFlowError.localMetadata(error).description)
                }
                return
            }
            // Already finished is idempotent success and cleans metadata left
            // by older plugin versions that finished StoreKit before deletion.
            do {
                try await localTransactionMetadata.remove(
                    transactionId: transactionId,
                    purchaseToken: args.purchaseToken
                )
                invoke.resolve()
            } catch {
                invoke.reject(PurchaseFlowError.localMetadata(error).description)
            }
        }
    }

    // MARK: - Mapping

    private static func productPayload(for product: Product) -> ProductPayload {
        ProductPayload(
            identifier: product.id,
            productType: pluginProductType(product.type),
            title: product.displayName,
            description: product.description,
            priceMicros: priceMicros(product.price),
            currency: product.priceFormatStyle.currencyCode,
            period: isoPeriod(product.subscription?.subscriptionPeriod)
        )
    }

    private static func purchasePayload(
        for transaction: Transaction,
        jwsRepresentation: String,
        receiptContext: ReceiptContext? = nil
    ) -> PurchasePayload {
        PurchasePayload(
            purchaseToken: jwsRepresentation,
            productIds: [transaction.productID],
            transactionId: String(transaction.id),
            purchaseTimeMs: Int64(transaction.purchaseDate.timeIntervalSince1970 * 1000),
            receiptContext: receiptContext
        )
    }

    private func unfinishedPurchasePayloads() async throws -> [PurchasePayload] {
        var accumulator = PurchasePayloadAccumulator()
        // Includes Ask-to-Buy approvals, unfinished consumables, and other
        // verified transactions delivered outside an explicit purchase call.
        for await result in Transaction.unfinished {
            guard case .verified(let transaction) = result else {
                continue
            }
            let receiptContext = try await receiptContext(
                for: transaction,
                jwsRepresentation: result.jwsRepresentation
            )
            accumulator = accumulator.appending(
                Self.purchasePayload(
                    for: transaction,
                    jwsRepresentation: result.jwsRepresentation,
                    receiptContext: receiptContext
                )
            )
        }
        return accumulator.payloads
    }

    /// Loads exact durable attribution, or promotes a same-process Ask-to-Buy
    /// context to the exact transaction identity before exposing it to Rust.
    /// A failed promotion throws so Rust cannot post and finish without the
    /// offering / placement / targeting context that is still recoverable.
    private func receiptContext(
        for transaction: Transaction,
        jwsRepresentation: String
    ) async throws -> ReceiptContext? {
        let transactionReasonIsRenewal: Bool?
        if #available(iOS 17.0, macOS 14.0, *) {
            transactionReasonIsRenewal = transaction.reason == .renewal
        } else {
            transactionReasonIsRenewal = nil
        }
        return try await receiptContextForSync(
            metadataStore: localTransactionMetadata,
            pendingContexts: pendingPurchaseContexts,
            transactionId: String(transaction.id),
            purchaseToken: jwsRepresentation,
            productId: transaction.productID,
            purchaseDate: transaction.purchaseDate,
            transactionReasonIsRenewal: transactionReasonIsRenewal
        )
    }

    @MainActor
    private func notifyTransactionUpdated() {
        trigger(nativeTransactionUpdatedEvent, data: JSObject())
    }

    private static func priceMicros(_ price: Decimal) -> Int64 {
        // Stay in Decimal to avoid binary floating-point drift on prices.
        NSDecimalNumber(decimal: price)
            .multiplying(byPowerOf10: 6)
            .int64Value
    }

    private static func isoPeriod(_ period: Product.SubscriptionPeriod?) -> String? {
        guard let period else {
            return nil
        }
        switch period.unit {
        case .day:
            return "P\(period.value)D"
        case .week:
            return "P\(period.value)W"
        case .month:
            return "P\(period.value)M"
        case .year:
            return "P\(period.value)Y"
        @unknown default:
            return nil
        }
    }
}

@available(iOS 15.0, macOS 12.0, *)
func pluginProductType(_ productType: Product.ProductType) -> String {
    switch productType {
    case .consumable:
        return "consumable"
    case .nonConsumable:
        return "non_consumable"
    case .autoRenewable, .nonRenewable:
        return "subscription"
    default:
        return "unknown"
    }
}

func finishPayloadMatches(
    expectedTransactionId: String,
    expectedPurchaseToken: String,
    actualTransactionId: String,
    actualPurchaseToken: String
) -> Bool {
    expectedTransactionId == actualTransactionId &&
        expectedPurchaseToken == actualPurchaseToken
}

func requestedProductOrder(_ identifiers: [String]) -> [String: Int] {
    identifiers.enumerated().reduce(into: [:]) { order, element in
        if order[element.element] == nil {
            order[element.element] = element.offset
        }
    }
}

func sandboxEnvironment(
    isSimulator: Bool,
    receiptURLPath: String?
) -> Bool? {
    if isSimulator {
        return true
    }
    return receiptURLPath.map { $0.contains("sandboxReceipt") }
}

private func currentSandboxEnvironment() -> Bool? {
#if targetEnvironment(simulator)
    let isSimulator = true
#else
    let isSimulator = false
#endif
    return sandboxEnvironment(
        isSimulator: isSimulator,
        receiptURLPath: Bundle.main.appStoreReceiptURL?.path
    )
}

// Registered on runtimes below the StoreKit 2 floor: every command is
// rejected instead of crashing at class load.
class UnsupportedRevenueCatPlugin: Plugin {
    @objc public func isSandboxEnvironment(_ invoke: Invoke) throws {
        invoke.resolve(
            SandboxEnvironmentResponse(
                isSandbox: currentSandboxEnvironment()
            )
        )
    }

    @objc public func queryProducts(_ invoke: Invoke) throws { rejectAll(invoke) }
    @objc public func purchase(_ invoke: Invoke) throws { rejectAll(invoke) }
    @objc public func queryPurchases(_ invoke: Invoke) throws { rejectAll(invoke) }
    @objc public func queryPendingPurchases(_ invoke: Invoke) throws { rejectAll(invoke) }
    @objc public func finishTransaction(_ invoke: Invoke) throws { rejectAll(invoke) }

    private func rejectAll(_ invoke: Invoke) {
        invoke.reject("StoreKit 2 requires iOS 15 / macOS 12 or newer.")
    }
}

@_cdecl("init_plugin_revenuecat")
public func initPlugin() -> Plugin {
    if #available(iOS 15.0, macOS 12.0, *) {
        return RevenueCatPlugin()
    }
    return UnsupportedRevenueCatPlugin()
}
