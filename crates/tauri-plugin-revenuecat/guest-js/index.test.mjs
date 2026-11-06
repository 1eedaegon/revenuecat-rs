import assert from "node:assert/strict";
import { mock, test } from "node:test";

const calls = [];
let syncResolvers = [];
let purchaseFailure;
let transactionUpdated;
const lifecycleHandlers = new Map();

mock.module("@tauri-apps/api/core", {
  exports: {
    addPluginListener: async (_plugin, _event, handler) => {
      transactionUpdated = handler;
      return { unregister: async () => undefined };
    },
    invoke: async (command) => {
      calls.push(command);
      if (command.endsWith("|configure")) {
        return {
          configured: true,
          appUserId: "user-a",
          isAnonymous: false,
          store: "play store",
        };
      }
      if (
        command.endsWith("|sync_pending_purchases") ||
        command.endsWith("|sync_purchases")
      ) {
        return await new Promise((resolve) => syncResolvers.push(resolve));
      }
      if (command.endsWith("|purchase_package")) {
        if (purchaseFailure) throw purchaseFailure;
        return { transaction: {}, customer_info: {} };
      }
      if (command.endsWith("|log_in")) {
        return { customerInfo: {}, created: false };
      }
      if (command.endsWith("|log_out")) return {};
      throw new Error(`Unexpected command: ${command}`);
    },
  },
});

mock.module("@tauri-apps/api/event", {
  exports: {
    emit: async () => undefined,
    listen: async (event, handler) => {
      lifecycleHandlers.set(event, handler);
      return async () => undefined;
    },
    TauriEvent: {
      WINDOW_RESUMED: "tauri://window-resumed",
      WINDOW_FOCUS: "tauri://focus",
    },
  },
});

const sdk = await import("./index.ts");
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function purchaseCallCount() {
  return calls.filter((command) => command.endsWith("|purchase_package")).length;
}

function finishNextSync() {
  const resolve = syncResolvers.shift();
  assert.equal(typeof resolve, "function");
  resolve({});
}

async function assertStartupRecovery() {
  await sdk.configure({ apiKey: "goog_test" });
  await tick();
  assert.equal(syncResolvers.length, 1);
  assert.ok(
    calls.includes("plugin:revenuecat|sync_purchases"),
    "Play startup must recover acknowledged active purchases too",
  );

  const firstPurchase = sdk.purchasePackage("monthly");
  await tick();
  assert.equal(
    purchaseCallCount(),
    0,
    "store UI must wait for startup queue reconciliation",
  );

  finishNextSync();
  await firstPurchase;
}

async function assertPurchaseFailureRecovery() {
  purchaseFailure = new Error("backend post failed after native purchase");
  await assert.rejects(sdk.purchasePackage("monthly"), purchaseFailure);
  await tick();
  assert.equal(syncResolvers.length, 1);

  purchaseFailure = undefined;
  const retry = sdk.purchasePackage("monthly");
  await tick();
  assert.equal(
    purchaseCallCount(),
    2,
    "retry must not reopen store UI before pending recovery succeeds",
  );

  finishNextSync();
  await retry;
  assert.equal(purchaseCallCount(), 3);
}

async function assertTransactionEventQueue() {
  transactionUpdated();
  await tick();
  assert.equal(syncResolvers.length, 1);
  transactionUpdated();
  finishNextSync();
  await tick();
  assert.equal(syncResolvers.length, 1);

  const purchaseCallsBeforeFirstSync = purchaseCallCount();
  const purchaseAfterQueuedUpdate = sdk.purchasePackage("monthly");
  await tick();
  assert.equal(
    purchaseCallCount(),
    purchaseCallsBeforeFirstSync,
    "a newer native update must keep purchase blocked after an older sync succeeds",
  );

  finishNextSync();
  await purchaseAfterQueuedUpdate;
  assert.equal(purchaseCallCount(), purchaseCallsBeforeFirstSync + 1);
}

async function assertLifecycleEventQueue() {
  lifecycleHandlers.get("tauri://window-resumed")();
  await tick();
  assert.equal(syncResolvers.length, 1);
  lifecycleHandlers.get("tauri://focus")();
  finishNextSync();
  await tick();
  assert.equal(
    syncResolvers.length,
    1,
    "focus received during resume sync must queue another reconciliation",
  );
  finishNextSync();
  await tick();
}

async function assertIdentityRecoveryBarrier() {
  transactionUpdated();
  await tick();
  const login = sdk.logIn("user-b");
  const logout = sdk.logOut();
  await tick();
  assert.equal(
    calls.filter(
      (command) =>
        command.endsWith("|log_in") || command.endsWith("|log_out"),
    ).length,
    0,
    "identity changes must wait for native receipt recovery",
  );

  finishNextSync();
  await Promise.all([login, logout]);
}

test("purchase waits for startup recovery and recovery after a native-flow error", async () => {
  await assertStartupRecovery();
  await assertPurchaseFailureRecovery();
  await assertTransactionEventQueue();
  await assertLifecycleEventQueue();
  await assertIdentityRecoveryBarrier();
});
