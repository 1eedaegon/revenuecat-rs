import assert from "node:assert/strict";
import { mock, test } from "node:test";

const calls = [];
const emittedEvents = [];
const scheduledTimers = [];
const pendingSyncResolvers = [];
let configuredStore = "app store";
let finalSyncFailure = new Error("sync failed");
let syncMode = "pending";
let syncAttempt = 0;
let transactionUpdated;

mock.method(globalThis, "setTimeout", (callback, delay = 0, ...args) => {
  const timer = {
    cleared: false,
    delay,
    run: () => callback(...args),
  };
  scheduledTimers.push(timer);
  return timer;
});

mock.method(globalThis, "clearTimeout", (timer) => {
  timer.cleared = true;
});

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
          store: configuredStore,
        };
      }
      if (
        command.endsWith("|sync_pending_purchases") ||
        command.endsWith("|sync_purchases")
      ) {
        if (syncMode === "fail") {
          syncAttempt += 1;
          throw finalSyncFailure;
        }
        return await new Promise((resolve) => pendingSyncResolvers.push(resolve));
      }
      if (command.endsWith("|purchase_package")) {
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
    emit: async (event, payload) => emittedEvents.push({ event, payload }),
    listen: async () => async () => undefined,
    TauriEvent: {
      WINDOW_RESUMED: "tauri://window-resumed",
      WINDOW_FOCUS: "tauri://focus",
    },
  },
});

const sdk = await import("./index.ts");

async function flushTasks() {
  await Promise.resolve();
  await new Promise((resolve) => setImmediate(resolve));
  await Promise.resolve();
}

async function runTimer(delay) {
  const timer = scheduledTimers.find(
    (candidate) => !candidate.cleared && candidate.delay === delay,
  );
  assert.ok(timer, `expected an active ${delay}ms timer`);
  timer.cleared = true;
  timer.run();
  await flushTasks();
}

function commandCallCount(suffix) {
  return calls.filter((command) => command.endsWith(suffix)).length;
}

function assertProtectedCommandCounts(expected) {
  assert.equal(commandCallCount("|purchase_package"), expected);
  assert.equal(commandCallCount("|log_in"), expected);
  assert.equal(commandCallCount("|log_out"), expected);
}

function finishNextSync() {
  const resolve = pendingSyncResolvers.shift();
  assert.equal(typeof resolve, "function");
  resolve({});
}

async function assertAppStoreUsesPendingSync() {
  configuredStore = "app store";
  syncMode = "pending";
  await sdk.configure({ apiKey: "appl_test" });
  await flushTasks();
  assert.equal(calls.at(-1), "plugin:revenuecat|sync_pending_purchases");
  assert.equal(commandCallCount("|sync_purchases"), 0);
  assert.equal(pendingSyncResolvers.length, 1);
  finishNextSync();
  await flushTasks();
}

async function assertPlayExhaustionFailsClosed() {
  configuredStore = "play store";
  syncMode = "fail";
  syncAttempt = 0;
  finalSyncFailure = {
    code: "SYNC_BACKEND_UNAVAILABLE",
    message: "native backend unavailable",
  };
  await sdk.configure({ apiKey: "goog_test" });
  await flushTasks();

  const recoveryError = {
    message:
      "A native purchase still requires synchronization; retry after native sync succeeds.",
  };
  const blockedOperations = [
    assert.rejects(sdk.purchasePackage("monthly"), recoveryError),
    assert.rejects(sdk.logIn("user-b"), recoveryError),
    assert.rejects(sdk.logOut(), recoveryError),
  ];
  await flushTasks();
  assertProtectedCommandCounts(0);

  await runTimer(750);
  await runTimer(3_000);
  await Promise.all(blockedOperations);
  assertProtectedCommandCounts(0);
  assert.equal(syncAttempt, 3);
  assert.deepEqual(emittedEvents, [
    {
      event: sdk.NATIVE_SYNC_ERROR_EVENT,
      payload: { message: "native backend unavailable" },
    },
  ]);
  assert.ok(
    scheduledTimers.some(
      (timer) => !timer.cleared && timer.delay === 15_000,
    ),
    "exhaustion must schedule a background retry",
  );
}

async function assertBackgroundFullSyncRecovers() {
  syncMode = "pending";
  await runTimer(15_000);
  assert.equal(calls.at(-1), "plugin:revenuecat|sync_purchases");
  assert.equal(pendingSyncResolvers.length, 1);
  finishNextSync();
  await flushTasks();

  await Promise.all([
    sdk.purchasePackage("monthly"),
    sdk.logIn("user-b"),
    sdk.logOut(),
  ]);
  assertProtectedCommandCounts(1);
}

async function exhaustPendingSyncWith(failure) {
  syncMode = "fail";
  syncAttempt = 0;
  finalSyncFailure = failure;
  transactionUpdated();
  await flushTasks();
  await runTimer(750);
  await runTimer(3_000);
  assert.equal(syncAttempt, 3);
  return emittedEvents.at(-1);
}

async function assertConservativeErrorFallbacks() {
  const secretFailure = {
    code: "PRIVATE_FAILURE",
    accessToken: "must-not-be-emitted",
  };
  const fallbackEvent = await exhaustPendingSyncWith(secretFailure);
  assert.deepEqual(fallbackEvent, {
    event: sdk.NATIVE_SYNC_ERROR_EVENT,
    payload: { message: "Native synchronization failed." },
  });
  assert.doesNotMatch(JSON.stringify(fallbackEvent), /must-not-be-emitted/);

  assert.deepEqual(await exhaustPendingSyncWith(503), {
    event: sdk.NATIVE_SYNC_ERROR_EVENT,
    payload: { message: "503" },
  });
}

test("native recovery retries fail closed and resume through the background timer", async () => {
  await assertAppStoreUsesPendingSync();
  await assertPlayExhaustionFailsClosed();
  await assertBackgroundFullSyncRecovers();
  await assertConservativeErrorFallbacks();
});
