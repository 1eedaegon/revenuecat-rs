import Foundation
import XCTest
@testable import tauri_plugin_revenuecat

final class PendingPurchaseContextClaimTests: XCTestCase {
    func testConcurrentSameTransactionWaitsForSingleDurablePromotion() async throws {
        let store = GatedLocalTransactionMetadataStore(shouldFail: false)
        let cache = PendingPurchaseContextCache()
        let context = receiptContext(offeringIdentifier: "original")
        let purchaseDate = Date(timeIntervalSince1970: 1_700_000_001)
        await cache.cache(
            context,
            productId: "monthly",
            purchaseAttemptDate: purchaseDate.addingTimeInterval(-1)
        )

        let first = syncTask(store: store, cache: cache, purchaseDate: purchaseDate)
        await store.waitUntilStoreStarts()
        let second = syncTask(store: store, cache: cache, purchaseDate: purchaseDate)
        await store.waitForReceiptLookupCount(2)
        let waiterRegistered = await waitForPromotionWaiter(cache)

        await store.releaseStores()
        let resolved = try await [first.value, second.value]
        let snapshot = await store.snapshot()

        XCTAssertTrue(waiterRegistered)
        XCTAssertEqual(resolved, [context, context])
        XCTAssertEqual(snapshot.storeCallCount, 1)
        XCTAssertEqual(snapshot.metadataByTransactionId.count, 1)
        XCTAssertEqual(snapshot.metadataByTransactionId["42"]?.receiptContext, context)
    }

    func testConcurrentSameTransactionObservesPromotionFailureAndRollsBack() async throws {
        let store = GatedLocalTransactionMetadataStore(shouldFail: true)
        let cache = PendingPurchaseContextCache()
        let context = receiptContext(offeringIdentifier: "original")
        let purchaseDate = Date(timeIntervalSince1970: 1_700_000_001)
        await cache.cache(
            context,
            productId: "monthly",
            purchaseAttemptDate: purchaseDate.addingTimeInterval(-1)
        )

        let first = syncFailureTask(store: store, cache: cache, purchaseDate: purchaseDate)
        await store.waitUntilStoreStarts()
        let second = syncFailureTask(store: store, cache: cache, purchaseDate: purchaseDate)
        await store.waitForReceiptLookupCount(2)
        let waiterRegistered = await waitForPromotionWaiter(cache)

        await store.releaseStores()
        let didFail = await [first.value, second.value]
        let failedSnapshot = await store.snapshot()
        let rolledBack = await cache.context(
            productId: "monthly",
            purchaseDate: purchaseDate
        )

        XCTAssertTrue(waiterRegistered)
        XCTAssertEqual(didFail, [true, true])
        XCTAssertEqual(failedSnapshot.storeCallCount, 1)
        XCTAssertTrue(failedSnapshot.metadataByTransactionId.isEmpty)
        XCTAssertEqual(rolledBack, context)

        await store.setShouldFail(false)
        let retried = try await sync(
            store: store,
            cache: cache,
            purchaseDate: purchaseDate
        )
        let retrySnapshot = await store.snapshot()
        XCTAssertEqual(retried, context)
        XCTAssertEqual(retrySnapshot.storeCallCount, 2)
        XCTAssertEqual(retrySnapshot.metadataByTransactionId.count, 1)
    }

    func testPendingContextCanOnlyBeClaimedOnce() async {
        let cache = PendingPurchaseContextCache()
        let purchaseDate = Date(timeIntervalSince1970: 1_700_000_001)
        await cache.cache(
            receiptContext(offeringIdentifier: "original"),
            productId: "monthly",
            purchaseAttemptDate: purchaseDate.addingTimeInterval(-1)
        )

        async let first = cache.claim(productId: "monthly", purchaseDate: purchaseDate)
        async let second = cache.claim(productId: "monthly", purchaseDate: purchaseDate)
        let claims = await [first, second]

        XCTAssertEqual(claims.compactMap { $0 }.count, 1)
    }

    func testClaimRollbackRestoresPendingContext() async throws {
        let cache = PendingPurchaseContextCache()
        let context = receiptContext(offeringIdentifier: "original")
        let purchaseDate = Date(timeIntervalSince1970: 1_700_000_001)
        await cache.cache(
            context,
            productId: "monthly",
            purchaseAttemptDate: purchaseDate.addingTimeInterval(-1)
        )
        let pendingClaim = await cache.claim(
            productId: "monthly",
            purchaseDate: purchaseDate
        )
        let claim = try XCTUnwrap(pendingClaim)

        await cache.rollback(claim)

        let restored = await cache.context(
            productId: "monthly",
            purchaseDate: purchaseDate
        )
        XCTAssertEqual(restored, context)
    }

    func testOldClaimCompletionNeverChangesANewerAttempt() async throws {
        let purchaseDate = Date(timeIntervalSince1970: 1_700_000_001)
        let oldContext = receiptContext(offeringIdentifier: "old")
        let newContext = receiptContext(offeringIdentifier: "new")

        let rollbackCache = PendingPurchaseContextCache()
        await rollbackCache.cache(
            oldContext,
            productId: "monthly",
            purchaseAttemptDate: purchaseDate.addingTimeInterval(-2)
        )
        let pendingRollbackClaim = await rollbackCache.claim(
            productId: "monthly",
            purchaseDate: purchaseDate
        )
        let rollbackClaim = try XCTUnwrap(pendingRollbackClaim)
        await rollbackCache.cache(
            newContext,
            productId: "monthly",
            purchaseAttemptDate: purchaseDate.addingTimeInterval(-1)
        )
        await rollbackCache.rollback(rollbackClaim)

        let commitCache = PendingPurchaseContextCache()
        await commitCache.cache(
            oldContext,
            productId: "monthly",
            purchaseAttemptDate: purchaseDate.addingTimeInterval(-2)
        )
        let pendingCommitClaim = await commitCache.claim(
            productId: "monthly",
            purchaseDate: purchaseDate
        )
        let commitClaim = try XCTUnwrap(pendingCommitClaim)
        await commitCache.cache(
            newContext,
            productId: "monthly",
            purchaseAttemptDate: purchaseDate.addingTimeInterval(-1)
        )
        await commitCache.commit(commitClaim)

        let afterRollback = await rollbackCache.context(
            productId: "monthly",
            purchaseDate: purchaseDate
        )
        let afterCommit = await commitCache.context(
            productId: "monthly",
            purchaseDate: purchaseDate
        )
        XCTAssertEqual(afterRollback, newContext)
        XCTAssertEqual(afterCommit, newContext)
    }

    func testConcurrentSyncPromotesPendingContextOnlyOnce() async throws {
        let store = GatedLocalTransactionMetadataStore(shouldFail: false)
        let cache = PendingPurchaseContextCache()
        let context = receiptContext(offeringIdentifier: "original")
        let purchaseDate = Date(timeIntervalSince1970: 1_700_000_001)
        await cache.cache(
            context,
            productId: "monthly",
            purchaseAttemptDate: purchaseDate.addingTimeInterval(-1)
        )

        let first = syncTask(store: store, cache: cache, purchaseDate: purchaseDate)
        await store.waitUntilStoreStarts()
        let second = Task {
            try await sync(
                store: store,
                cache: cache,
                purchaseDate: purchaseDate,
                transactionId: "43",
                purchaseToken: "jws-43"
            )
        }
        await store.waitForReceiptLookupCount(2)
        await store.releaseStores()

        let resolved = try await [first.value, second.value]
        let snapshot = await store.snapshot()
        let unrelatedWaiterCount = await cache.promotionWaiterCount(
            transactionId: "43",
            purchaseToken: "jws-43",
            productId: "monthly"
        )

        XCTAssertEqual(resolved.compactMap { $0 }.count, 1)
        XCTAssertEqual(resolved.compactMap { $0 }.first, context)
        XCTAssertEqual(snapshot.storeCallCount, 1)
        XCTAssertEqual(snapshot.metadataByTransactionId.count, 1)
        XCTAssertEqual(snapshot.metadataByTransactionId["42"]?.receiptContext, context)
        XCTAssertNil(snapshot.metadataByTransactionId["43"])
        XCTAssertEqual(unrelatedWaiterCount, 0)
    }

    private func receiptContext(offeringIdentifier: String) -> ReceiptContext {
        ReceiptContext(
            presentedOfferingIdentifier: offeringIdentifier,
            presentedPlacementIdentifier: nil,
            appliedTargetingRule: nil,
            priceMicros: nil,
            currency: nil,
            normalDuration: nil,
            sdkOriginated: true
        )
    }

    private func syncTask(
        store: any LocalTransactionMetadataStoring,
        cache: PendingPurchaseContextCache,
        purchaseDate: Date
    ) -> Task<ReceiptContext?, Error> {
        Task {
            try await sync(store: store, cache: cache, purchaseDate: purchaseDate)
        }
    }

    private func syncFailureTask(
        store: any LocalTransactionMetadataStoring,
        cache: PendingPurchaseContextCache,
        purchaseDate: Date
    ) -> Task<Bool, Never> {
        Task {
            do {
                _ = try await sync(
                    store: store,
                    cache: cache,
                    purchaseDate: purchaseDate
                )
                return false
            } catch {
                return true
            }
        }
    }

    private func sync(
        store: any LocalTransactionMetadataStoring,
        cache: PendingPurchaseContextCache,
        purchaseDate: Date,
        transactionId: String = "42",
        purchaseToken: String = "jws-42"
    ) async throws -> ReceiptContext? {
        try await receiptContextForSync(
            metadataStore: store,
            pendingContexts: cache,
            transactionId: transactionId,
            purchaseToken: purchaseToken,
            productId: "monthly",
            purchaseDate: purchaseDate,
            transactionReasonIsRenewal: false
        )
    }

    private func waitForPromotionWaiter(
        _ cache: PendingPurchaseContextCache
    ) async -> Bool {
        for _ in 0..<1_000 {
            if await cache.promotionWaiterCount(
                transactionId: "42",
                purchaseToken: "jws-42",
                productId: "monthly"
            ) == 1 {
                return true
            }
            await Task.yield()
        }
        return false
    }
}

private enum GatedMetadataStoreError: Error {
    case forcedFailure
}

private struct GatedMetadataStoreSnapshot: Sendable {
    let storeCallCount: Int
    let metadataByTransactionId: [String: LocalTransactionMetadata]
}

private actor GatedLocalTransactionMetadataStore: LocalTransactionMetadataStoring {
    private var shouldFail: Bool
    private var isReleased = false
    private var storeCallCount = 0
    private var receiptLookupCount = 0
    private var metadataByTransactionId: [String: LocalTransactionMetadata] = [:]
    private var storeReleaseWaiters: [CheckedContinuation<Void, Never>] = []
    private var storeStartWaiters: [CheckedContinuation<Void, Never>] = []
    private var receiptLookupWaiters: [ReceiptLookupWaiter] = []

    init(shouldFail: Bool) {
        self.shouldFail = shouldFail
    }

    func store(_ metadata: LocalTransactionMetadata) async throws {
        storeCallCount += 1
        let startWaiters = storeStartWaiters
        storeStartWaiters = []
        startWaiters.forEach { $0.resume() }
        if !isReleased {
            await withCheckedContinuation { continuation in
                storeReleaseWaiters.append(continuation)
            }
        }
        if shouldFail {
            throw GatedMetadataStoreError.forcedFailure
        }
        metadataByTransactionId[metadata.transactionId] = metadata
    }

    func receiptContext(
        transactionId: String,
        purchaseToken: String,
        productId: String
    ) async throws -> ReceiptContext? {
        receiptLookupCount += 1
        resumeSatisfiedReceiptLookupWaiters()
        guard let metadata = metadataByTransactionId[transactionId],
              metadata.purchaseToken == purchaseToken,
              metadata.productId == productId else {
            return nil
        }
        return metadata.receiptContext
    }

    func waitUntilStoreStarts() async {
        if storeCallCount > 0 {
            return
        }
        await withCheckedContinuation { continuation in
            storeStartWaiters.append(continuation)
        }
    }

    func waitForReceiptLookupCount(_ expectedCount: Int) async {
        if receiptLookupCount >= expectedCount {
            return
        }
        await withCheckedContinuation { continuation in
            receiptLookupWaiters.append(
                ReceiptLookupWaiter(expectedCount: expectedCount, continuation: continuation)
            )
        }
    }

    func releaseStores() {
        isReleased = true
        let waiters = storeReleaseWaiters
        storeReleaseWaiters = []
        waiters.forEach { $0.resume() }
    }

    func setShouldFail(_ shouldFail: Bool) {
        self.shouldFail = shouldFail
    }

    func snapshot() -> GatedMetadataStoreSnapshot {
        GatedMetadataStoreSnapshot(
            storeCallCount: storeCallCount,
            metadataByTransactionId: metadataByTransactionId
        )
    }

    private func resumeSatisfiedReceiptLookupWaiters() {
        let satisfied = receiptLookupWaiters.filter {
            receiptLookupCount >= $0.expectedCount
        }
        receiptLookupWaiters.removeAll {
            receiptLookupCount >= $0.expectedCount
        }
        satisfied.forEach { $0.continuation.resume() }
    }
}

private struct ReceiptLookupWaiter {
    let expectedCount: Int
    let continuation: CheckedContinuation<Void, Never>
}
