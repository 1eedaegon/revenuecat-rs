import assert from "node:assert/strict";
import { mock, test } from "node:test";

const syncCalls = [];
const syncResolvers = [];
let transactionUpdated;

mock.module("@tauri-apps/api/core", {
  exports: {
    addPluginListener: async (_plugin, _event, handler) => {
      transactionUpdated = handler;
      return { unregister: async () => undefined };
    },
    invoke: async (command) => {
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
        syncCalls.push(command);
        return await new Promise((resolve) => syncResolvers.push(resolve));
      }
      throw new Error(`Unexpected command: ${command}`);
    },
  },
});

mock.module("@tauri-apps/api/event", {
  exports: {
    emit: async () => undefined,
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

function finishNextSync() {
  const resolve = syncResolvers.shift();
  assert.equal(typeof resolve, "function");
  resolve({});
}

test("a full Play recovery queued during pending sync is never downgraded", async () => {
  await sdk.configure({ apiKey: "goog_test" });
  await flushTasks();
  assert.deepEqual(syncCalls, ["plugin:revenuecat|sync_purchases"]);
  finishNextSync();
  await flushTasks();

  transactionUpdated();
  await flushTasks();
  assert.deepEqual(syncCalls, [
    "plugin:revenuecat|sync_purchases",
    "plugin:revenuecat|sync_pending_purchases",
  ]);

  await sdk.configure({ apiKey: "goog_test" });
  finishNextSync();
  await flushTasks();
  assert.deepEqual(syncCalls, [
    "plugin:revenuecat|sync_purchases",
    "plugin:revenuecat|sync_pending_purchases",
    "plugin:revenuecat|sync_purchases",
  ]);

  finishNextSync();
  await flushTasks();
  assert.equal(syncResolvers.length, 0);
});
