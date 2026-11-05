import assert from "node:assert/strict";
import { mock, test } from "node:test";

const lifecycleError = new Error("focus listener unavailable");
const pluginError = new Error("plugin listener unavailable");
let rejectFirstPluginListener;
let pluginListenerAttempts = 0;
let resumedListenerAttempts = 0;
let focusListenerAttempts = 0;
let partialCleanupCalls = 0;
const invokeCalls = [];

mock.module("@tauri-apps/api/core", {
  exports: {
    addPluginListener: (plugin, event) => {
      assert.equal(plugin, "revenuecat");
      assert.equal(event, "transaction-updated");
      pluginListenerAttempts += 1;
      if (pluginListenerAttempts === 1) {
        return new Promise((_resolve, reject) => {
          rejectFirstPluginListener = reject;
        });
      }
      return Promise.resolve({ unregister: async () => undefined });
    },
    invoke: async (command, args) => {
      invokeCalls.push({ command, args });
      if (command === "plugin:revenuecat|configure") {
        return {
          configured: true,
          appUserId: "user-a",
          isAnonymous: false,
          store: "play store",
        };
      }
      if (command === "plugin:revenuecat|sync_purchases") return {};
      throw new Error(`Unexpected command: ${command}`);
    },
  },
});

mock.module("@tauri-apps/api/event", {
  exports: {
    emit: async () => undefined,
    listen: async (event) => {
      if (event === "tauri://window-resumed") {
        resumedListenerAttempts += 1;
        if (resumedListenerAttempts === 1) {
          return async () => {
            partialCleanupCalls += 1;
            throw new Error("cleanup failure must not mask listener failure");
          };
        }
        return async () => undefined;
      }
      if (event === "tauri://focus") {
        focusListenerAttempts += 1;
        if (focusListenerAttempts === 1) throw lifecycleError;
        return async () => undefined;
      }
      throw new Error(`Unexpected event: ${event}`);
    },
    TauriEvent: {
      WINDOW_RESUMED: "tauri://window-resumed",
      WINDOW_FOCUS: "tauri://focus",
    },
  },
});

const sdk = await import("./index.ts");
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

test("partial lifecycle and plugin listener failures clean up and are retryable", async () => {
  const options = { apiKey: "goog_test" };
  const firstConfigure = sdk.configure(options);

  await assert.rejects(firstConfigure, lifecycleError);
  assert.equal(partialCleanupCalls, 1);
  assert.equal(typeof rejectFirstPluginListener, "function");
  rejectFirstPluginListener(pluginError);
  await tick();

  const session = await sdk.configure(options);
  await tick();
  assert.equal(session.store, "play store");
  assert.equal(pluginListenerAttempts, 2);
  assert.equal(resumedListenerAttempts, 2);
  assert.equal(focusListenerAttempts, 2);
  assert.equal(partialCleanupCalls, 1);
  assert.deepEqual(
    invokeCalls.map(({ command }) => command),
    [
      "plugin:revenuecat|configure",
      "plugin:revenuecat|configure",
      "plugin:revenuecat|sync_purchases",
    ],
  );
});
