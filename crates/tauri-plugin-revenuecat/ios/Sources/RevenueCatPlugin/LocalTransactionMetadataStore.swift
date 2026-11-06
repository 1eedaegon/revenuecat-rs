import Foundation

struct ReceiptTargetingRule: Codable, Equatable, Sendable {
    let ruleId: String
    let revision: Int64
}

/// Receipt fields supplied by Rust when an SDK purchase is launched.
///
/// The same shape is returned on the native channel if StoreKit delivers the
/// transaction again after the original process has exited.
struct ReceiptContext: Codable, Equatable, Sendable {
    let presentedOfferingIdentifier: String?
    let presentedPlacementIdentifier: String?
    let appliedTargetingRule: ReceiptTargetingRule?
    let priceMicros: Int64?
    let currency: String?
    let normalDuration: String?
    let sdkOriginated: Bool
}

struct LocalTransactionMetadata: Codable, Equatable, Sendable {
    let transactionId: String
    let purchaseToken: String
    let productId: String
    let receiptContext: ReceiptContext
}

protocol LocalTransactionMetadataStoring: Sendable {
    func store(_ metadata: LocalTransactionMetadata) async throws

    func receiptContext(
        transactionId: String,
        purchaseToken: String,
        productId: String
    ) async throws -> ReceiptContext?
}

/// Orders the irreversible StoreKit acknowledgement after durable metadata
/// cleanup. The caller has already received backend acceptance, so a crash
/// after cleanup is recoverable through an idempotent receipt re-post; the
/// reverse order can leave an unqueryable JWS in the journal forever.
func removeMetadataThenFinishTransaction(
    removeMetadata: () async throws -> Void,
    finishTransaction: () async -> Void
) async throws {
    try await removeMetadata()
    await finishTransaction()
}

/// A crash-safe journal for receipt attribution that already has an exact
/// StoreKit transaction identity.
///
/// This intentionally does not persist pre-transaction Ask-to-Buy intents:
/// before approval StoreKit provides neither a transaction ID nor a JWS, and
/// product-ID-only persistence can attribute a renewal or an external purchase
/// to the wrong offering. `PendingPurchaseContextCache` covers the same-process
/// deferred path, matching purchases-ios's correlation boundary.
actor LocalTransactionMetadataStore: LocalTransactionMetadataStoring {
    private static let maximumJournalBytes = 128 * 1_024
    private static let maximumTransactions = 32

    private struct MetadataFile: Codable {
        let version: Int
        var transactions: [String: LocalTransactionMetadata]

        init(transactions: [String: LocalTransactionMetadata]) {
            version = 1
            self.transactions = transactions
        }
    }

    private let fileURL: URL
    private let fileManager: FileManager
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()

    init(
        fileURL: URL = LocalTransactionMetadataStore.defaultFileURL(),
        fileManager: FileManager = .default
    ) {
        self.fileURL = fileURL
        self.fileManager = fileManager
        encoder.outputFormatting = [.sortedKeys]
    }

    func store(_ metadata: LocalTransactionMetadata) async throws {
        var file = try load()
        if let existing = file.transactions[metadata.transactionId] {
            guard existing == metadata else {
                throw CocoaError(.fileWriteFileExists)
            }
            return
        }
        file.transactions[metadata.transactionId] = metadata
        try write(file)
    }

    func receiptContext(
        transactionId: String,
        purchaseToken: String
    ) throws -> ReceiptContext? {
        guard let metadata = try load().transactions[transactionId],
              metadata.purchaseToken == purchaseToken else {
            return nil
        }
        return metadata.receiptContext
    }

    func receiptContext(
        transactionId: String,
        purchaseToken: String,
        productId: String
    ) async throws -> ReceiptContext? {
        guard let metadata = try load().transactions[transactionId],
              metadata.purchaseToken == purchaseToken,
              metadata.productId == productId else {
            return nil
        }
        return metadata.receiptContext
    }

    /// Removes metadata only when both StoreKit identities match. Returning
    /// false is an idempotent no-op and never deletes another transaction's
    /// attribution.
    @discardableResult
    func remove(transactionId: String, purchaseToken: String) throws -> Bool {
        var file = try load()
        guard file.transactions[transactionId]?.purchaseToken == purchaseToken else {
            return false
        }
        file.transactions.removeValue(forKey: transactionId)
        try write(file)
        return true
    }

    private func load() throws -> MetadataFile {
        guard fileManager.fileExists(atPath: fileURL.path) else {
            return MetadataFile(transactions: [:])
        }
        let resourceValues = try fileURL.resourceValues(forKeys: [.fileSizeKey])
        guard let fileSize = resourceValues.fileSize,
              fileSize <= Self.maximumJournalBytes else {
            throw CocoaError(.fileReadCorruptFile)
        }
        let data = try Data(contentsOf: fileURL)
        guard data.count <= Self.maximumJournalBytes else {
            throw CocoaError(.fileReadCorruptFile)
        }
        let file = try decoder.decode(MetadataFile.self, from: data)
        guard file.version == 1,
              file.transactions.count <= Self.maximumTransactions,
              file.transactions.allSatisfy({ $0.key == $0.value.transactionId }) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        return file
    }

    private func write(_ file: MetadataFile) throws {
        guard file.transactions.count <= Self.maximumTransactions else {
            throw CocoaError(.fileWriteUnknown)
        }
        let data = try encoder.encode(file)
        guard data.count <= Self.maximumJournalBytes else {
            throw CocoaError(.fileWriteUnknown)
        }
        try fileManager.createDirectory(
            at: fileURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try data.write(
            to: fileURL,
            options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication]
        )
    }

    private static func defaultFileURL() -> URL {
        let root = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.temporaryDirectory
        return root
            .appendingPathComponent("app.tauri.revenuecat", isDirectory: true)
            .appendingPathComponent("storekit2-transaction-metadata-v1.json", isDirectory: false)
    }
}

private struct PendingPurchaseContext: Sendable {
    let attemptId: UUID
    let receiptContext: ReceiptContext
    let purchaseAttemptDate: Date
}

private struct PendingPurchaseTransactionIdentity: Hashable, Sendable {
    let transactionId: String
    let purchaseToken: String
    let productId: String
}

private enum PendingPurchaseContextAcquisition: Sendable {
    case owner(PendingPurchaseContextClaim)
    case retryDurableMetadata
    case absent
}

struct PendingPurchaseContextClaim: Sendable {
    fileprivate let productId: String
    fileprivate let attemptId: UUID
    fileprivate let receiptContext: ReceiptContext
    fileprivate let purchaseAttemptDate: Date
    fileprivate let transactionIdentity: PendingPurchaseTransactionIdentity?
}

/// Correlates an Ask-to-Buy approval while this process remains alive.
actor PendingPurchaseContextCache {
    private var contextsByProductId: [String: PendingPurchaseContext] = [:]
    private var latestAttemptIdsByProductId: [String: UUID] = [:]
    private var promotionAttemptIdsByIdentity: [PendingPurchaseTransactionIdentity: UUID] = [:]
    private var promotionWaiters: [
        PendingPurchaseTransactionIdentity: [CheckedContinuation<Void, Error>]
    ] = [:]

    func cache(
        _ context: ReceiptContext,
        productId: String,
        purchaseAttemptDate: Date = Date()
    ) {
        let attemptId = UUID()
        latestAttemptIdsByProductId[productId] = attemptId
        contextsByProductId[productId] = PendingPurchaseContext(
            attemptId: attemptId,
            receiptContext: context,
            purchaseAttemptDate: purchaseAttemptDate
        )
    }

    func context(productId: String, purchaseDate: Date) -> ReceiptContext? {
        guard let cached = contextsByProductId[productId],
              cached.purchaseAttemptDate <= purchaseDate else {
            return nil
        }
        return cached.receiptContext
    }

    func claim(
        productId: String,
        purchaseDate: Date
    ) -> PendingPurchaseContextClaim? {
        claim(
            productId: productId,
            purchaseDate: purchaseDate,
            transactionIdentity: nil
        )
    }

    fileprivate func claimOrWaitForPromotion(
        transactionId: String,
        purchaseToken: String,
        productId: String,
        purchaseDate: Date
    ) async throws -> PendingPurchaseContextAcquisition {
        let identity = PendingPurchaseTransactionIdentity(
            transactionId: transactionId,
            purchaseToken: purchaseToken,
            productId: productId
        )
        if promotionAttemptIdsByIdentity[identity] != nil {
            try await waitForPromotion(identity)
            return .retryDurableMetadata
        }
        guard let claim = claim(
            productId: productId,
            purchaseDate: purchaseDate,
            transactionIdentity: identity
        ) else {
            return .absent
        }
        promotionAttemptIdsByIdentity[identity] = claim.attemptId
        return .owner(claim)
    }

    private func claim(
        productId: String,
        purchaseDate: Date,
        transactionIdentity: PendingPurchaseTransactionIdentity?
    ) -> PendingPurchaseContextClaim? {
        guard let cached = contextsByProductId[productId],
              cached.purchaseAttemptDate <= purchaseDate else {
            return nil
        }
        contextsByProductId.removeValue(forKey: productId)
        return PendingPurchaseContextClaim(
            productId: productId,
            attemptId: cached.attemptId,
            receiptContext: cached.receiptContext,
            purchaseAttemptDate: cached.purchaseAttemptDate,
            transactionIdentity: transactionIdentity
        )
    }

    func commit(_ claim: PendingPurchaseContextClaim) {
        completePromotion(for: claim, error: nil)
        guard latestAttemptIdsByProductId[claim.productId] == claim.attemptId else {
            return
        }
        if contextsByProductId[claim.productId]?.attemptId == claim.attemptId {
            contextsByProductId.removeValue(forKey: claim.productId)
        }
        latestAttemptIdsByProductId.removeValue(forKey: claim.productId)
    }

    func rollback(_ claim: PendingPurchaseContextClaim, error: Error? = nil) {
        if latestAttemptIdsByProductId[claim.productId] == claim.attemptId,
           contextsByProductId[claim.productId] == nil {
            contextsByProductId[claim.productId] = PendingPurchaseContext(
                attemptId: claim.attemptId,
                receiptContext: claim.receiptContext,
                purchaseAttemptDate: claim.purchaseAttemptDate
            )
        }
        completePromotion(
            for: claim,
            error: error ?? CocoaError(.fileWriteUnknown)
        )
    }

    func remove(productId: String) {
        contextsByProductId.removeValue(forKey: productId)
        latestAttemptIdsByProductId.removeValue(forKey: productId)
    }

    func promotionWaiterCount(
        transactionId: String,
        purchaseToken: String,
        productId: String
    ) -> Int {
        let identity = PendingPurchaseTransactionIdentity(
            transactionId: transactionId,
            purchaseToken: purchaseToken,
            productId: productId
        )
        return promotionWaiters[identity]?.count ?? 0
    }

    private func waitForPromotion(
        _ identity: PendingPurchaseTransactionIdentity
    ) async throws {
        try await withCheckedThrowingContinuation { continuation in
            promotionWaiters[identity, default: []].append(continuation)
        }
    }

    private func completePromotion(
        for claim: PendingPurchaseContextClaim,
        error: Error?
    ) {
        guard let identity = claim.transactionIdentity,
              promotionAttemptIdsByIdentity[identity] == claim.attemptId else {
            return
        }
        promotionAttemptIdsByIdentity.removeValue(forKey: identity)
        let waiters = promotionWaiters.removeValue(forKey: identity) ?? []
        waiters.forEach { continuation in
            if let error {
                continuation.resume(throwing: error)
            } else {
                continuation.resume()
            }
        }
    }
}

private func durableReceiptContext(
    metadataStore: any LocalTransactionMetadataStoring,
    transactionId: String,
    purchaseToken: String,
    productId: String
) async throws -> ReceiptContext? {
    try await metadataStore.receiptContext(
        transactionId: transactionId,
        purchaseToken: purchaseToken,
        productId: productId
    )
}

private func promotePendingReceiptContext(
    metadataStore: any LocalTransactionMetadataStoring,
    pendingContexts: PendingPurchaseContextCache,
    claim: PendingPurchaseContextClaim,
    transactionId: String,
    purchaseToken: String,
    productId: String
) async throws -> ReceiptContext {
    do {
        try await metadataStore.store(
            LocalTransactionMetadata(
                transactionId: transactionId,
                purchaseToken: purchaseToken,
                productId: productId,
                receiptContext: claim.receiptContext
            )
        )
        await pendingContexts.commit(claim)
        return claim.receiptContext
    } catch {
        await pendingContexts.rollback(claim, error: error)
        throw error
    }
}

private func resolvePendingReceiptContext(
    _ acquisition: PendingPurchaseContextAcquisition,
    metadataStore: any LocalTransactionMetadataStoring,
    pendingContexts: PendingPurchaseContextCache,
    transactionId: String,
    purchaseToken: String,
    productId: String
) async throws -> ReceiptContext? {
    switch acquisition {
    case .owner(let claim):
        return try await promotePendingReceiptContext(
            metadataStore: metadataStore,
            pendingContexts: pendingContexts,
            claim: claim,
            transactionId: transactionId,
            purchaseToken: purchaseToken,
            productId: productId
        )
    case .retryDurableMetadata:
        guard let persisted = try await durableReceiptContext(
            metadataStore: metadataStore,
            transactionId: transactionId,
            purchaseToken: purchaseToken,
            productId: productId
        ) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        return persisted
    case .absent:
        return try await durableReceiptContext(
            metadataStore: metadataStore,
            transactionId: transactionId,
            purchaseToken: purchaseToken,
            productId: productId
        )
    }
}

/// Returns durable attribution for queue sync. A same-process pending context
/// is exposed only after it has been promoted to an exact transaction ID/JWS;
/// persistence failures propagate so the caller cannot finish contextlessly.
func receiptContextForSync(
    metadataStore: any LocalTransactionMetadataStoring,
    pendingContexts: PendingPurchaseContextCache,
    transactionId: String,
    purchaseToken: String,
    productId: String,
    purchaseDate: Date,
    transactionReasonIsRenewal: Bool?
) async throws -> ReceiptContext? {
    if let persisted = try await durableReceiptContext(
        metadataStore: metadataStore,
        transactionId: transactionId,
        purchaseToken: purchaseToken,
        productId: productId
    ) {
        return persisted
    }

    guard shouldUsePendingPurchaseContext(
        transactionReasonIsRenewal: transactionReasonIsRenewal
    ) else {
        return nil
    }

    let acquisition = try await pendingContexts.claimOrWaitForPromotion(
        transactionId: transactionId,
        purchaseToken: purchaseToken,
        productId: productId,
        purchaseDate: purchaseDate
    )
    return try await resolvePendingReceiptContext(
        acquisition,
        metadataStore: metadataStore,
        pendingContexts: pendingContexts,
        transactionId: transactionId,
        purchaseToken: purchaseToken,
        productId: productId
    )
}
