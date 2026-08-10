import Foundation
import StoreKit
import Tauri
import WebKit

// MARK: - Command arguments

private struct QueryProductsArgs: Decodable {
    let ids: [String]
}

private struct PurchaseArgs: Decodable {
    let productId: String
}

private struct FinishTransactionArgs: Decodable {
    let transactionId: String
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

private struct PurchasePayload: Encodable {
    let purchaseToken: String
    let productIds: [String]
    let transactionId: String
    let purchaseTimeMs: Int64
}

private struct PurchasesResponse: Encodable {
    let purchases: [PurchasePayload]
}

// MARK: - Errors

private enum PurchaseFlowError: Error, CustomStringConvertible {
    case productNotFound(String)
    case cancelled
    case pending
    case alreadyInFlight
    case transactionNotFound(String)
    case storeKit(Error)

    var description: String {
        switch self {
        case .productNotFound(let id):
            return "Product not found: \(id)"
        case .cancelled:
            return "cancelled"
        case .pending:
            return "pending"
        case .alreadyInFlight:
            return "A purchase is already in progress"
        case .transactionNotFound(let id):
            return "Unfinished transaction not found: \(id)"
        case .storeKit(let error):
            return "StoreKit error: \(error.localizedDescription)"
        }
    }
}

// MARK: - Purchase gate

private actor PurchaseGate {
    private var isInFlight = false

    func begin() -> Bool {
        if isInFlight {
            return false
        }
        isInFlight = true
        return true
    }

    func end() {
        isInFlight = false
    }

    var inFlight: Bool { isInFlight }
}

// MARK: - Plugin

class RevenueCatPlugin: Plugin {

    private let purchaseGate = PurchaseGate()
    private var updatesTask: Task<Void, Never>?

    deinit {
        updatesTask?.cancel()
    }

    @objc public override func load(webview: WKWebView) {
        // `Transaction.updates` delivers renewals, Ask-to-Buy approvals, and purchases
        // made outside this app instance. Unfinished transactions are redelivered by
        // StoreKit on every launch until finished, so orphans are finished here.
        // Entitlement state is recovered from `Transaction.currentEntitlements`
        // (which includes finished transactions), not from this stream.
        updatesTask?.cancel()
        updatesTask = Task.detached(priority: .utility) { [purchaseGate] in
            for await result in Transaction.updates {
                // Never race the explicit purchase flow: its transaction is finished
                // by the `finishTransaction` command after backend acknowledgment.
                if await purchaseGate.inFlight {
                    continue
                }
                guard case .verified(let transaction) = result else {
                    continue
                }
                await transaction.finish()
            }
        }
    }

    // MARK: queryProducts

    @objc public func queryProducts(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(QueryProductsArgs.self)
        Task {
            do {
                let products = try await Product.products(for: args.ids)
                // StoreKit returns products unordered; restore request order.
                let order = Dictionary(
                    uniqueKeysWithValues: args.ids.enumerated().map { ($1, $0) }
                )
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
                outcome = .success(try await self.performPurchase(productId: args.productId))
            } catch let error as PurchaseFlowError {
                outcome = .failure(error)
            } catch {
                outcome = .failure(.storeKit(error))
            }
            await self.purchaseGate.end()

            switch outcome {
            case .success(let payload):
                invoke.resolve(payload)
            case .failure(let error):
                invoke.reject(error.description)
            }
        }
    }

    private func performPurchase(productId: String) async throws -> PurchasePayload {
        guard let product = try await Product.products(for: [productId]).first else {
            throw PurchaseFlowError.productNotFound(productId)
        }

        let result: Product.PurchaseResult
        do {
            result = try await product.purchase()
        } catch {
            throw PurchaseFlowError.storeKit(error)
        }

        switch result {
        case .success(let verification):
            // The JWS is passed through as the purchase token without local
            // re-verification: the backend validates the signature. The transaction
            // stays unfinished until the backend acknowledges (`finishTransaction`).
            let transaction = verification.unsafePayloadValue
            return Self.purchasePayload(
                for: transaction,
                jwsRepresentation: verification.jwsRepresentation
            )
        case .pending:
            // Ask to Buy / SCA: the transaction arrives later via Transaction.updates.
            throw PurchaseFlowError.pending
        case .userCancelled:
            throw PurchaseFlowError.cancelled
        @unknown default:
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
            var payloads: [PurchasePayload] = []
            // currentEntitlements yields the latest transaction per active
            // entitlement (subscriptions + non-consumables); consumables never
            // appear here once finished.
            for await result in Transaction.currentEntitlements {
                guard case .verified(let transaction) = result else {
                    continue
                }
                payloads.append(
                    Self.purchasePayload(
                        for: transaction,
                        jwsRepresentation: result.jwsRepresentation
                    )
                )
            }
            invoke.resolve(PurchasesResponse(purchases: payloads))
        }
    }

    // MARK: finishTransaction

    @objc public func finishTransaction(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(FinishTransactionArgs.self)
        Task {
            for await result in Transaction.unfinished {
                let transaction = result.unsafePayloadValue
                guard String(transaction.id) == args.transactionId else {
                    continue
                }
                // StoreKit 2 has no client-side consume; for consumables,
                // finish() is what removes the transaction from the queue.
                // `shouldConsume` is accepted for cross-platform contract parity.
                _ = args.shouldConsume
                await transaction.finish()
                invoke.resolve()
                return
            }
            // Already finished (e.g. by the updates listener) is not an error:
            // the desired end state holds either way.
            invoke.resolve()
        }
    }

    // MARK: - Mapping

    private static func productPayload(for product: Product) -> ProductPayload {
        ProductPayload(
            identifier: product.id,
            productType: product.type == .consumable ? "consumable" : "subscription",
            title: product.displayName,
            description: product.description,
            priceMicros: priceMicros(product.price),
            currency: product.priceFormatStyle.currencyCode,
            period: isoPeriod(product.subscription?.subscriptionPeriod)
        )
    }

    private static func purchasePayload(
        for transaction: Transaction,
        jwsRepresentation: String
    ) -> PurchasePayload {
        PurchasePayload(
            purchaseToken: jwsRepresentation,
            productIds: [transaction.productID],
            transactionId: String(transaction.id),
            purchaseTimeMs: Int64(transaction.purchaseDate.timeIntervalSince1970 * 1000)
        )
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

@_cdecl("init_plugin_revenuecat")
func initPlugin() -> Plugin {
    return RevenueCatPlugin()
}
