import Foundation
import XCTest
@testable import tauri_plugin_revenuecat

final class LocalTransactionMetadataStoreTests: XCTestCase {
    func testMetadataSurvivesStoreRecreationAndRequiresExactJWS() async throws {
        let fileURL = temporaryFileURL()
        let metadata = makeMetadata()

        let writer = LocalTransactionMetadataStore(fileURL: fileURL)
        try await writer.store(metadata)

        let reader = LocalTransactionMetadataStore(fileURL: fileURL)
        let restored = try await reader.receiptContext(
            transactionId: metadata.transactionId,
            purchaseToken: metadata.purchaseToken
        )
        let wrongToken = try await reader.receiptContext(
            transactionId: metadata.transactionId,
            purchaseToken: "different-jws"
        )

        XCTAssertEqual(restored, metadata.receiptContext)
        XCTAssertNil(wrongToken)
    }

    func testWrongJWSCannotDeleteMetadata() async throws {
        let fileURL = temporaryFileURL()
        let metadata = makeMetadata()
        let store = LocalTransactionMetadataStore(fileURL: fileURL)
        try await store.store(metadata)

        let removed = try await store.remove(
            transactionId: metadata.transactionId,
            purchaseToken: "different-jws"
        )

        let retained = try await store.receiptContext(
            transactionId: metadata.transactionId,
            purchaseToken: metadata.purchaseToken
        )

        XCTAssertFalse(removed)
        XCTAssertEqual(retained, metadata.receiptContext)
    }

    func testExactFinishDeletesMetadataDurably() async throws {
        let fileURL = temporaryFileURL()
        let metadata = makeMetadata()
        let store = LocalTransactionMetadataStore(fileURL: fileURL)
        try await store.store(metadata)

        let removed = try await store.remove(
            transactionId: metadata.transactionId,
            purchaseToken: metadata.purchaseToken
        )

        XCTAssertTrue(removed)
        let reloaded = LocalTransactionMetadataStore(fileURL: fileURL)
        let restored = try await reloaded.receiptContext(
            transactionId: metadata.transactionId,
            purchaseToken: metadata.purchaseToken
        )
        XCTAssertNil(restored)
    }

    func testMetadataRemovalPrecedesStoreKitFinish() async throws {
        let recorder = AsyncOperationRecorder()

        try await removeMetadataThenFinishTransaction(
            removeMetadata: {
                await recorder.record("remove-metadata")
            },
            finishTransaction: {
                await recorder.record("finish-storekit")
            }
        )

        let operations = await recorder.snapshot()
        XCTAssertEqual(operations, ["remove-metadata", "finish-storekit"])
    }

    func testMetadataRemovalFailureLeavesStoreKitUnfinished() async {
        let recorder = AsyncOperationRecorder()

        do {
            try await removeMetadataThenFinishTransaction(
                removeMetadata: {
                    await recorder.record("remove-metadata")
                    throw ExpectedRemovalError.failure
                },
                finishTransaction: {
                    await recorder.record("finish-storekit")
                }
            )
            XCTFail("metadata removal failure should propagate")
        } catch ExpectedRemovalError.failure {
            // Expected: StoreKit must remain unfinished for a later retry.
        } catch {
            XCTFail("unexpected error: \(error)")
        }

        let operations = await recorder.snapshot()
        XCTAssertEqual(operations, ["remove-metadata"])
    }

    func testPendingContextOnlyMatchesATransactionCreatedAfterTheAttempt() async {
        let cache = PendingPurchaseContextCache()
        let context = makeReceiptContext()
        let attemptDate = Date(timeIntervalSince1970: 1_700_000_000)
        await cache.cache(
            context,
            productId: "monthly",
            purchaseAttemptDate: attemptDate
        )

        let beforeAttempt = await cache.context(
            productId: "monthly",
            purchaseDate: attemptDate.addingTimeInterval(-1)
        )
        let afterAttempt = await cache.context(
            productId: "monthly",
            purchaseDate: attemptDate.addingTimeInterval(1)
        )
        await cache.remove(productId: "monthly")
        let removed = await cache.context(
            productId: "monthly",
            purchaseDate: attemptDate.addingTimeInterval(1)
        )

        XCTAssertNil(beforeAttempt)
        XCTAssertEqual(afterAttempt, context)
        XCTAssertNil(removed)
    }

    func testPendingContextIsDurableBeforeQueueSyncReturnsIt() async throws {
        let store = LocalTransactionMetadataStore(fileURL: temporaryFileURL())
        let cache = PendingPurchaseContextCache()
        let context = makeReceiptContext()
        let purchaseDate = Date(timeIntervalSince1970: 1_700_000_001)
        await cache.cache(
            context,
            productId: "monthly",
            purchaseAttemptDate: purchaseDate.addingTimeInterval(-1)
        )

        let resolved = try await receiptContextForSync(
            metadataStore: store,
            pendingContexts: cache,
            transactionId: "42",
            purchaseToken: "jws-42",
            productId: "monthly",
            purchaseDate: purchaseDate,
            transactionReasonIsRenewal: false
        )
        let persisted = try await store.receiptContext(
            transactionId: "42",
            purchaseToken: "jws-42"
        )
        let pending = await cache.context(
            productId: "monthly",
            purchaseDate: purchaseDate
        )

        XCTAssertEqual(resolved, context)
        XCTAssertEqual(persisted, context)
        XCTAssertNil(pending)
    }

    func testRenewalDoesNotPromotePendingContext() async throws {
        let store = LocalTransactionMetadataStore(fileURL: temporaryFileURL())
        let cache = PendingPurchaseContextCache()
        let context = makeReceiptContext()
        let purchaseDate = Date(timeIntervalSince1970: 1_700_000_001)
        await cache.cache(
            context,
            productId: "monthly",
            purchaseAttemptDate: purchaseDate.addingTimeInterval(-1)
        )

        let resolved = try await receiptContextForSync(
            metadataStore: store,
            pendingContexts: cache,
            transactionId: "42",
            purchaseToken: "renewal-jws",
            productId: "monthly",
            purchaseDate: purchaseDate,
            transactionReasonIsRenewal: true
        )
        let pending = await cache.context(
            productId: "monthly",
            purchaseDate: purchaseDate
        )

        XCTAssertNil(resolved)
        XCTAssertEqual(pending, context)
    }

    func testPromotionFailurePropagatesAndRetainsPendingContext() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(
            at: root,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: root) }
        let blockingParent = root.appendingPathComponent("not-a-directory")
        try Data().write(to: blockingParent)

        let store = LocalTransactionMetadataStore(
            fileURL: blockingParent.appendingPathComponent("metadata.json")
        )
        let cache = PendingPurchaseContextCache()
        let context = makeReceiptContext()
        let purchaseDate = Date(timeIntervalSince1970: 1_700_000_001)
        await cache.cache(
            context,
            productId: "monthly",
            purchaseAttemptDate: purchaseDate.addingTimeInterval(-1)
        )

        do {
            _ = try await receiptContextForSync(
                metadataStore: store,
                pendingContexts: cache,
                transactionId: "42",
                purchaseToken: "jws-42",
                productId: "monthly",
                purchaseDate: purchaseDate,
                transactionReasonIsRenewal: false
            )
            XCTFail("queue sync must fail when attribution cannot be persisted")
        } catch {
            // Expected: callers reject instead of posting without attribution.
        }

        let pending = await cache.context(
            productId: "monthly",
            purchaseDate: purchaseDate
        )
        XCTAssertEqual(pending, context)
    }

    func testReceiptContextUsesTheSharedCamelCaseNativeWire() throws {
        let payload = PurchasePayload(
            purchaseToken: "jws-42",
            productIds: ["monthly"],
            transactionId: "42",
            purchaseTimeMs: 1_700_000_000_000,
            receiptContext: makeReceiptContext()
        )

        let encoded = try JSONEncoder().encode(payload)
        let json = try XCTUnwrap(
            JSONSerialization.jsonObject(with: encoded) as? [String: Any]
        )
        let context = try XCTUnwrap(json["receiptContext"] as? [String: Any])
        let targeting = try XCTUnwrap(
            context["appliedTargetingRule"] as? [String: Any]
        )

        XCTAssertEqual(context["presentedOfferingIdentifier"] as? String, "default")
        XCTAssertEqual(context["presentedPlacementIdentifier"] as? String, "onboarding")
        XCTAssertEqual(context["priceMicros"] as? Int, 4_990_000)
        XCTAssertEqual(context["currency"] as? String, "USD")
        XCTAssertEqual(context["normalDuration"] as? String, "P1M")
        XCTAssertEqual(context["sdkOriginated"] as? Bool, true)
        XCTAssertEqual(targeting["ruleId"] as? String, "rule")
        XCTAssertEqual(targeting["revision"] as? Int, 7)
    }

    func testRenewalsNeverConsumePendingPurchaseContextWhenReasonIsAvailable() {
        XCTAssertFalse(
            shouldUsePendingPurchaseContext(transactionReasonIsRenewal: true)
        )
        XCTAssertTrue(
            shouldUsePendingPurchaseContext(transactionReasonIsRenewal: false)
        )
        XCTAssertTrue(
            shouldUsePendingPurchaseContext(transactionReasonIsRenewal: nil),
            "iOS versions before Transaction.reason retain the product/date fallback"
        )
    }

    private func makeMetadata() -> LocalTransactionMetadata {
        LocalTransactionMetadata(
            transactionId: "42",
            purchaseToken: "jws-42",
            productId: "monthly",
            receiptContext: makeReceiptContext()
        )
    }

    private func makeReceiptContext() -> ReceiptContext {
        ReceiptContext(
            presentedOfferingIdentifier: "default",
            presentedPlacementIdentifier: "onboarding",
            appliedTargetingRule: ReceiptTargetingRule(ruleId: "rule", revision: 7),
            priceMicros: 4_990_000,
            currency: "USD",
            normalDuration: "P1M",
            sdkOriginated: true
        )
    }

    private func temporaryFileURL() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
            .appendingPathComponent("metadata.json", isDirectory: false)
    }
}

private enum ExpectedRemovalError: Error {
    case failure
}

private actor AsyncOperationRecorder {
    private var operations: [String] = []

    func record(_ operation: String) {
        operations.append(operation)
    }

    func snapshot() -> [String] {
        operations
    }
}
