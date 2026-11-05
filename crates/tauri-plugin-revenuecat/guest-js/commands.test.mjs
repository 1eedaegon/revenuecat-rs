import assert from "node:assert/strict";
import { mock, test } from "node:test";

const calls = [];
const listenedEvents = [];
let pluginListenerCalls = 0;

const session = {
  configured: true,
  appUserId: "user-a",
  isAnonymous: false,
  store: "test store",
};
const offering = { identifier: "default", packages: [] };
const offeringsResponses = [
  { current_offering_id: "default", all: { default: offering } },
  { current_offering_id: "missing", all: {} },
  { current_offering_id: "__proto__", all: {} },
  { current_offering_id: null, all: {} },
];
const customerInfo = { marker: "customer-info" };
const purchaseResult = { marker: "purchase-result" };
const restoredInfo = { marker: "restored-info" };
const loginResult = { marker: "login-result" };
const logoutInfo = { marker: "logout-info" };
const configureOptions = {
  apiKey: "test_key",
  appUserId: "user-a",
  proxyUrl: "http://127.0.0.1:1234",
  entitlementVerificationMode: "enforced",
  verificationRootKey: "root-key",
  dangerouslyAllowTestStoreInReleaseBuilds: true,
};

mock.module("@tauri-apps/api/core", {
  exports: {
    addPluginListener: async () => {
      pluginListenerCalls += 1;
      throw new Error("test-store configure must not install a native listener");
    },
    invoke: async (command, args) => {
      calls.push({ command, args });
      switch (command) {
        case "plugin:revenuecat|configure":
        case "plugin:revenuecat|session_info":
          return session;
        case "plugin:revenuecat|get_offerings":
          return offeringsResponses.shift();
        case "plugin:revenuecat|get_customer_info":
          return customerInfo;
        case "plugin:revenuecat|purchase_package":
          return purchaseResult;
        case "plugin:revenuecat|restore":
          return restoredInfo;
        case "plugin:revenuecat|log_in":
          return loginResult;
        case "plugin:revenuecat|log_out":
          return logoutInfo;
        case "plugin:revenuecat|set_email":
          return undefined;
        case "plugin:revenuecat|manage_subscriptions":
          return "https://example.test/manage";
        default:
          throw new Error(`Unexpected command: ${command}`);
      }
    },
  },
});

mock.module("@tauri-apps/api/event", {
  exports: {
    emit: async () => {
      throw new Error("test-store commands must not emit native sync errors");
    },
    listen: async (event, handler) => {
      const registration = {
        event,
        handler,
        unlisten: async () => undefined,
      };
      listenedEvents.push(registration);
      return registration.unlisten;
    },
    TauriEvent: {
      WINDOW_RESUMED: "tauri://window-resumed",
      WINDOW_FOCUS: "tauri://focus",
    },
  },
});

const sdk = await import("./index.ts");

async function assertConfigureAndOfferings() {
  assert.strictEqual(await sdk.configure(configureOptions), session);
  assert.equal(pluginListenerCalls, 0);
  assert.equal(listenedEvents.length, 0);
  assert.strictEqual(await sdk.sessionInfo(), session);

  const wire = offeringsResponses[0];
  const normalized = await sdk.getOfferings();
  assert.notStrictEqual(
    normalized,
    wire,
    "normalization must not mutate the IPC value",
  );
  assert.strictEqual(normalized.all, wire.all);
  assert.equal(normalized.current_offering_id, "default");
  assert.strictEqual(normalized.current, offering);
  assert.equal(Object.hasOwn(wire, "current"), false);

  for (const expectedId of ["missing", "__proto__", null]) {
    const value = await sdk.getOfferings();
    assert.equal(value.current_offering_id, expectedId);
    assert.equal(value.current, null, "only an own offering entry may become current");
  }
}

async function assertCommandResults() {
  assert.strictEqual(await sdk.getCustomerInfo(), customerInfo);
  assert.strictEqual(await sdk.purchasePackage("$rc_monthly"), purchaseResult);
  assert.strictEqual(await sdk.restore(), restoredInfo);
  assert.strictEqual(await sdk.logIn("user-b"), loginResult);
  assert.strictEqual(await sdk.logOut(), logoutInfo);
  assert.equal(await sdk.setEmail("user@example.test"), undefined);
  assert.equal(
    await sdk.manageSubscriptions(),
    "https://example.test/manage",
  );
}

async function assertListenerResults() {
  let customerPayload;
  let syncErrorPayload;
  const customerUnlisten = await sdk.onCustomerInfoUpdated((value) => {
    customerPayload = value;
  });
  const syncErrorUnlisten = await sdk.onNativeSyncError((value) => {
    syncErrorPayload = value;
  });
  assert.strictEqual(customerUnlisten, listenedEvents[0].unlisten);
  assert.strictEqual(syncErrorUnlisten, listenedEvents[1].unlisten);
  listenedEvents[0].handler({ payload: customerInfo });
  const nativeSyncError = { message: "offline" };
  listenedEvents[1].handler({ payload: nativeSyncError });
  assert.strictEqual(customerPayload, customerInfo);
  assert.strictEqual(syncErrorPayload, nativeSyncError);
  assert.deepEqual(
    listenedEvents.map(({ event }) => event),
    ["revenuecat:customer-info-updated", sdk.NATIVE_SYNC_ERROR_EVENT],
  );
}

const expectedCalls = [
  { command: "plugin:revenuecat|configure", args: { options: configureOptions } },
  { command: "plugin:revenuecat|session_info", args: undefined },
  ...Array.from({ length: 4 }, () => ({
    command: "plugin:revenuecat|get_offerings",
    args: undefined,
  })),
  { command: "plugin:revenuecat|get_customer_info", args: undefined },
  {
    command: "plugin:revenuecat|purchase_package",
    args: { packageId: "$rc_monthly" },
  },
  { command: "plugin:revenuecat|restore", args: undefined },
  { command: "plugin:revenuecat|log_in", args: { appUserId: "user-b" } },
  { command: "plugin:revenuecat|log_out", args: undefined },
  {
    command: "plugin:revenuecat|set_email",
    args: { email: "user@example.test" },
  },
  { command: "plugin:revenuecat|manage_subscriptions", args: undefined },
];

test("public wrappers preserve the wire contract and exact Tauri IPC surface", async () => {
  await assertConfigureAndOfferings();
  await assertCommandResults();
  await assertListenerResults();
  assert.deepEqual(calls, expectedCalls);
});
