import XCTest
@testable import tauri_plugin_revenuecat

final class PurchasePayloadAccumulatorTests: XCTestCase {
    func testNativeTransactionUpdateEventNameIsStable() {
        XCTAssertEqual(nativeTransactionUpdatedEvent, "transaction-updated")
    }

    func testMapsEveryKnownStoreKitProductType() {
        XCTAssertEqual(pluginProductType(.consumable), "consumable")
        XCTAssertEqual(pluginProductType(.nonConsumable), "non_consumable")
        XCTAssertEqual(pluginProductType(.autoRenewable), "subscription")
        XCTAssertEqual(pluginProductType(.nonRenewable), "subscription")
    }

    func testDuplicateProductIdsKeepTheirFirstRequestedOrderWithoutTrapping() {
        XCTAssertEqual(
            requestedProductOrder(["monthly", "monthly", "annual"]),
            ["monthly": 0, "annual": 2]
        )
    }

    func testFinishRequiresMatchingTransactionIdAndJWS() {
        XCTAssertTrue(
            finishPayloadMatches(
                expectedTransactionId: "42",
                expectedPurchaseToken: "jws-42",
                actualTransactionId: "42",
                actualPurchaseToken: "jws-42"
            )
        )
        XCTAssertFalse(
            finishPayloadMatches(
                expectedTransactionId: "42",
                expectedPurchaseToken: "jws-42",
                actualTransactionId: "42",
                actualPurchaseToken: "different-jws"
            )
        )
        XCTAssertFalse(
            finishPayloadMatches(
                expectedTransactionId: "42",
                expectedPurchaseToken: "jws-42",
                actualTransactionId: "43",
                actualPurchaseToken: "jws-42"
            )
        )
    }

    func testDeduplicatesPayloadsFromFullHistoryAndUnfinishedTransactions() {
        let historicalPurchase = makePayload(transactionId: "42")
        let unfinishedDuplicate = PurchasePayload(
            purchaseToken: "new-jws-for-42",
            productIds: ["same-transaction-different-source"],
            transactionId: "42",
            purchaseTimeMs: 1_700_000_000_001
        )
        let accumulator = PurchasePayloadAccumulator()
            .appending(unfinishedDuplicate)
            .appending(historicalPurchase)

        XCTAssertEqual(accumulator.payloads, [unfinishedDuplicate])
    }

    func testPreservesDistinctTransactionsInDiscoveryOrder() {
        let historicalPurchase = makePayload(transactionId: "42")
        let unfinishedConsumable = makePayload(transactionId: "43")
        let accumulator = PurchasePayloadAccumulator()
            .appending(historicalPurchase)
            .appending(unfinishedConsumable)

        XCTAssertEqual(accumulator.payloads, [historicalPurchase, unfinishedConsumable])
    }

    func testSimulatorIsAlwaysReportedAsSandbox() {
        XCTAssertEqual(
            sandboxEnvironment(isSimulator: true, receiptURLPath: nil),
            true
        )
    }

    func testDeviceSandboxReceiptIsReportedAsSandbox() {
        XCTAssertEqual(
            sandboxEnvironment(
                isSimulator: false,
                receiptURLPath: "/app/StoreKit/sandboxReceipt"
            ),
            true
        )
    }

    func testDeviceProductionReceiptIsNotReportedAsSandbox() {
        XCTAssertEqual(
            sandboxEnvironment(
                isSimulator: false,
                receiptURLPath: "/app/StoreKit/receipt"
            ),
            false
        )
    }

    func testDeviceWithoutReceiptHasUnknownEnvironment() {
        XCTAssertNil(
            sandboxEnvironment(isSimulator: false, receiptURLPath: nil)
        )
    }

    func testSandboxEnvironmentPayloadUsesCamelCaseKey() throws {
        let data = try JSONEncoder().encode(
            SandboxEnvironmentResponse(isSandbox: true)
        )
        let json = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: Bool]
        )

        XCTAssertEqual(json, ["isSandbox": true])
    }

    func testAppAccountTokenParsesAValidAppUserId() {
        XCTAssertEqual(
            appAccountToken(from: "123e4567-e89b-12d3-a456-426614174000"),
            UUID(uuidString: "123e4567-e89b-12d3-a456-426614174000")
        )
    }

    func testAppAccountTokenRejectsNonUUIDAndMissingAppUserIds() {
        XCTAssertNil(appAccountToken(from: "customer@example.com"))
        XCTAssertNil(appAccountToken(from: nil))
    }

    @available(iOS 15.0, macOS 12.0, *)
    func testPurchaseGateSuppressesAnExplicitUpdateDeliveredBeforeCompletion() async {
        let gate = PurchaseGate()

        let began = await gate.begin()
        let shouldNotify = await gate.shouldNotifyUpdate(transactionId: "42")
        let shouldNotifyOnEnd = await gate.end(explicitTransactionId: "42")

        XCTAssertTrue(began)
        XCTAssertFalse(shouldNotify)
        XCTAssertFalse(shouldNotifyOnEnd)
    }

    @available(iOS 15.0, macOS 12.0, *)
    func testPurchaseGateSuppressesAnExplicitUpdateDeliveredAfterCompletion() async {
        let gate = PurchaseGate()

        let began = await gate.begin()
        let shouldNotifyOnEnd = await gate.end(explicitTransactionId: "42")
        let shouldNotifyExplicit = await gate.shouldNotifyUpdate(transactionId: "42")
        let shouldNotifyRenewal = await gate.shouldNotifyUpdate(transactionId: "renewal")

        XCTAssertTrue(began)
        XCTAssertFalse(shouldNotifyOnEnd)
        XCTAssertFalse(shouldNotifyExplicit)
        XCTAssertTrue(shouldNotifyRenewal)
    }

    @available(iOS 15.0, macOS 12.0, *)
    func testPurchaseGateReleasesUnrelatedDeferredUpdates() async {
        let gate = PurchaseGate()

        let began = await gate.begin()
        let shouldNotifyDuringPurchase = await gate.shouldNotifyUpdate(transactionId: "renewal")
        let shouldNotifyOnEnd = await gate.end(explicitTransactionId: "42")

        XCTAssertTrue(began)
        XCTAssertFalse(shouldNotifyDuringPurchase)
        XCTAssertTrue(shouldNotifyOnEnd)
    }

    private func makePayload(transactionId: String) -> PurchasePayload {
        PurchasePayload(
            purchaseToken: "jws-\(transactionId)",
            productIds: ["product-\(transactionId)"],
            transactionId: transactionId,
            purchaseTimeMs: 1_700_000_000_000
        )
    }
}
