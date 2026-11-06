import Foundation
import XCTest
@testable import tauri_plugin_revenuecat

final class LocalTransactionMetadataStoreSecurityTests: XCTestCase {
    private static let maximumJournalBytes = 128 * 1_024
    private static let maximumTransactions = 32

    func testDifferentJWSCannotOverwriteTheSameTransaction() async throws {
        let fileURL = temporaryFileURL()
        let store = LocalTransactionMetadataStore(fileURL: fileURL)
        let original = metadata(
            purchaseToken: "original-jws",
            offeringIdentifier: "original-offering"
        )
        let conflicting = metadata(
            purchaseToken: "conflicting-jws",
            offeringIdentifier: "conflicting-offering"
        )
        try await store.store(original)

        do {
            try await store.store(conflicting)
            XCTFail("a transaction identity must not be rebound to a different JWS")
        } catch {
            // Expected: retain the first exact transaction identity.
        }

        let retained = try await store.receiptContext(
            transactionId: original.transactionId,
            purchaseToken: original.purchaseToken
        )
        let rejected = try await store.receiptContext(
            transactionId: conflicting.transactionId,
            purchaseToken: conflicting.purchaseToken
        )
        XCTAssertEqual(retained, original.receiptContext)
        XCTAssertNil(rejected)
    }

    func testExactMetadataIsIdempotentButOtherAttributionCannotOverwriteIt() async throws {
        let fileURL = temporaryFileURL()
        let store = LocalTransactionMetadataStore(fileURL: fileURL)
        let original = metadata(
            purchaseToken: "same-jws",
            offeringIdentifier: "original-offering"
        )
        try await store.store(original)
        let journalBeforeRetries = try Data(contentsOf: fileURL)

        try await store.store(original)
        XCTAssertEqual(try Data(contentsOf: fileURL), journalBeforeRetries)

        let conflicts = [
            metadata(
                purchaseToken: "same-jws",
                productId: "annual",
                offeringIdentifier: "original-offering"
            ),
            metadata(
                purchaseToken: "same-jws",
                offeringIdentifier: "different-offering"
            ),
        ]
        for conflict in conflicts {
            do {
                try await store.store(conflict)
                XCTFail("an exact transaction identity must not change attribution")
            } catch {
                // Expected: retain the first fully bound transaction metadata.
            }
            XCTAssertEqual(try Data(contentsOf: fileURL), journalBeforeRetries)
        }
    }

    func testJournalKeyAndPayloadTransactionIdMismatchFailsClosed() async throws {
        let fileURL = temporaryFileURL()
        try FileManager.default.createDirectory(
            at: fileURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let mismatchedJournal = #"""
        {
          "version": 1,
          "transactions": {
            "42": {
              "transactionId": "43",
              "purchaseToken": "jws-42",
              "productId": "monthly",
              "receiptContext": { "sdkOriginated": true }
            }
          }
        }
        """#
        try Data(mismatchedJournal.utf8).write(to: fileURL)
        let store = LocalTransactionMetadataStore(fileURL: fileURL)

        do {
            _ = try await store.receiptContext(
                transactionId: "42",
                purchaseToken: "jws-42"
            )
            XCTFail("a mismatched journal key must never provide attribution")
        } catch {
            // Expected: a structurally valid but inconsistent journal is corrupt.
        }
    }

    func testOversizedJournalWithValidEmptySchemaFailsClosed() async throws {
        let fileURL = temporaryFileURL()
        try FileManager.default.createDirectory(
            at: fileURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let padding = String(repeating: "x", count: Self.maximumJournalBytes)
        let journal = #"{"version":1,"transactions":{},"padding":"\#(padding)"}"#
        try Data(journal.utf8).write(to: fileURL)
        let store = LocalTransactionMetadataStore(fileURL: fileURL)

        do {
            _ = try await store.receiptContext(
                transactionId: "42",
                purchaseToken: "jws-42"
            )
            XCTFail("an oversized journal must not be treated as empty")
        } catch {
            // Expected: a valid header cannot bypass the byte limit.
        }
    }

    func testOversizedStoreFailsWithoutChangingValidJournal() async throws {
        let fileURL = temporaryFileURL()
        let store = LocalTransactionMetadataStore(fileURL: fileURL)
        let original = metadata(
            purchaseToken: "original-jws",
            offeringIdentifier: "original-offering"
        )
        try await store.store(original)
        let journalBeforeFailure = try Data(contentsOf: fileURL)
        let oversized = metadata(
            transactionId: "43",
            purchaseToken: String(repeating: "j", count: Self.maximumJournalBytes),
            offeringIdentifier: "oversized-offering"
        )

        do {
            try await store.store(oversized)
            XCTFail("an oversized encoded journal must not be written")
        } catch {
            // Expected: preserve the last valid journal.
        }

        XCTAssertEqual(try Data(contentsOf: fileURL), journalBeforeFailure)
        let retained = try await store.receiptContext(
            transactionId: original.transactionId,
            purchaseToken: original.purchaseToken
        )
        XCTAssertEqual(retained, original.receiptContext)
    }

    func testJournalOverTransactionLimitFailsClosed() async throws {
        let fileURL = temporaryFileURL()
        try FileManager.default.createDirectory(
            at: fileURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        var transactions: [String: Any] = [:]
        for index in 0...Self.maximumTransactions {
            let identifier = String(index)
            transactions[identifier] = serializedMetadata(transactionId: identifier)
        }
        let journal = try JSONSerialization.data(
            withJSONObject: ["version": 1, "transactions": transactions],
            options: [.sortedKeys]
        )
        try journal.write(to: fileURL)
        let store = LocalTransactionMetadataStore(fileURL: fileURL)

        do {
            _ = try await store.receiptContext(
                transactionId: "0",
                purchaseToken: "jws-0"
            )
            XCTFail("an over-capacity journal must not provide attribution")
        } catch {
            // Expected: bounded journal cardinality is part of its schema.
        }
    }

    func testThirtyThirdStoreFailsWithoutChangingValidJournal() async throws {
        let fileURL = temporaryFileURL()
        let store = LocalTransactionMetadataStore(fileURL: fileURL)
        for index in 0..<Self.maximumTransactions {
            try await store.store(
                metadata(
                    transactionId: String(index),
                    purchaseToken: "jws-\(index)",
                    offeringIdentifier: "offering-\(index)"
                )
            )
        }
        let journalBeforeFailure = try Data(contentsOf: fileURL)

        do {
            try await store.store(
                metadata(
                    transactionId: String(Self.maximumTransactions),
                    purchaseToken: "jws-over-capacity",
                    offeringIdentifier: "over-capacity"
                )
            )
            XCTFail("a thirty-third transaction must not be written")
        } catch {
            // Expected: preserve all 32 valid entries.
        }

        XCTAssertEqual(try Data(contentsOf: fileURL), journalBeforeFailure)
    }

    private func metadata(
        transactionId: String = "42",
        purchaseToken: String,
        productId: String = "monthly",
        offeringIdentifier: String
    ) -> LocalTransactionMetadata {
        LocalTransactionMetadata(
            transactionId: transactionId,
            purchaseToken: purchaseToken,
            productId: productId,
            receiptContext: ReceiptContext(
                presentedOfferingIdentifier: offeringIdentifier,
                presentedPlacementIdentifier: nil,
                appliedTargetingRule: nil,
                priceMicros: nil,
                currency: nil,
                normalDuration: nil,
                sdkOriginated: true
            )
        )
    }

    private func serializedMetadata(transactionId: String) -> [String: Any] {
        [
            "transactionId": transactionId,
            "purchaseToken": "jws-\(transactionId)",
            "productId": "monthly",
            "receiptContext": ["sdkOriginated": true],
        ]
    }

    private func temporaryFileURL() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
            .appendingPathComponent("metadata.json", isDirectory: false)
    }
}
