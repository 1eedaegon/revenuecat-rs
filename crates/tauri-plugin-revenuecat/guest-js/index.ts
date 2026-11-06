/**
 * Typed JS/TS bindings for `tauri-plugin-revenuecat`.
 *
 * Register the plugin in Rust (`.plugin(tauri_plugin_revenuecat::init())`),
 * then drive RevenueCat entirely from the webview:
 *
 * ```ts
 * import { configure, getOfferings, purchasePackage } from "tauri-plugin-revenuecat";
 *
 * await configure({ apiKey: "test_...", appUserId: "gon" });
 * const offerings = await getOfferings();
 * const pkg = offerings.current?.packages[0];
 * if (pkg) await purchasePackage(pkg.identifier);
 * ```
 *
 * Field casing matches the wire format: the SDK models (Offerings, CustomerInfo,
 * ...) are snake_case; the plugin's own envelope types (SessionInfo,
 * ConfigureOptions, LoginResult) are camelCase.
 */

import {
  addPluginListener,
  invoke,
  type PluginListener,
} from "@tauri-apps/api/core";
import {
  emit,
  listen,
  TauriEvent,
  type UnlistenFn,
} from "@tauri-apps/api/event";

// -- Enums (string-literal unions matching the serde output) ----------------

export type ProductType =
  | "subscription"
  | "consumable"
  | "non_consumable"
  | "unknown";

export type PeriodType = "normal" | "intro" | "trial" | "prepaid" | "unknown";

export type Store =
  | "app_store"
  | "mac_app_store"
  | "play_store"
  | "stripe"
  | "promotional"
  | "amazon"
  | "rc_billing"
  | "external"
  | "paddle"
  | "test_store"
  | "unknown";

export type OwnershipType = "PURCHASED" | "FAMILY_SHARED";

export type VerificationResult =
  | "NotRequested"
  | "Verified"
  | "VerifiedOnDevice"
  | "Failed";

/** Well-known `$rc_*` packages get a named type; anything else is `{ Custom }`. */
export type PackageType =
  | "Lifetime"
  | "Annual"
  | "SixMonth"
  | "ThreeMonth"
  | "TwoMonth"
  | "Monthly"
  | "Weekly"
  | { Custom: string };

// -- Products & offerings (snake_case) --------------------------------------

export interface Price {
  amount_micros: number;
  currency: string;
}

export interface PricingPhase {
  period_duration: string | null;
  cycle_count: number;
  price: Price | null;
}

export interface StoreProduct {
  identifier: string;
  product_type: ProductType;
  title: string;
  description: string | null;
  price: Price;
  subscription_period: string | null;
  trial: PricingPhase | null;
  intro_price: PricingPhase | null;
}

export interface Package {
  identifier: string;
  package_type: PackageType;
  store_product: StoreProduct;
  presented_offering_context: PresentedOfferingContext;
}

export interface PresentedOfferingContext {
  offering_identifier: string | null;
  placement_identifier: string | null;
  targeting: unknown | null;
}

export interface Offering {
  identifier: string;
  server_description: string | null;
  metadata: unknown | null;
  packages: Package[];
  /** Dashboard v1 template paywall — render it in your UI. */
  paywall: Paywall | null;
  /** v2 component-based paywall (raw config). */
  paywall_components: PaywallComponents | null;
}

export interface Offerings {
  /**
   * Raw Rust IPC field retained on SDK results. Optional so 1.x callers that
   * construct `{ all, current }` values remain source compatible.
   */
  current_offering_id?: string | null;
  /** Current offering resolved from `current_offering_id`, matching SDK ergonomics. */
  current: Offering | null;
  all: Record<string, Offering>;
}

interface OfferingsWire {
  current_offering_id: string | null;
  all: Record<string, Offering>;
}

// -- Paywall (v1 template config; see the crate for the full shape) ----------

export interface Paywall {
  template_name: string;
  revision: number;
  default_locale: string | null;
  asset_base_url: string | null;
  config: PaywallConfig;
  localized_strings: Record<string, PaywallLocalizedStrings>;
}

export interface PaywallConfig {
  packages: string[];
  default_package: string | null;
  images: PaywallImages;
  images_webp: PaywallImages;
  colors: { light: PaywallColors; dark: PaywallColors | null };
  title: string | null;
  subtitle: string | null;
  call_to_action: string | null;
  offer_details: string | null;
  features: PaywallFeature[];
  tos_url: string | null;
  privacy_url: string | null;
  display_restore_purchases: boolean;
}

export interface PaywallImages {
  header: string | null;
  background: string | null;
  icon: string | null;
}

export type PaywallColors = Record<string, string | null>;

export interface PaywallFeature {
  title: string;
  content: string | null;
  icon_id: string | null;
}

export interface PaywallLocalizedStrings {
  title: string | null;
  subtitle: string | null;
  call_to_action: string | null;
  offer_details: string | null;
  features: PaywallFeature[];
}

export interface PaywallComponents {
  template_name: string | null;
  revision: number;
  default_locale: string | null;
  asset_base_url: string | null;
  components_config: unknown;
  components_localizations: unknown;
}

// -- Customer info (snake_case) ---------------------------------------------

export interface EntitlementInfo {
  identifier: string;
  is_active: boolean;
  will_renew: boolean;
  period_type: PeriodType;
  latest_purchase_date: string | null;
  original_purchase_date: string | null;
  expiration_date: string | null;
  store: Store;
  product_identifier: string;
  product_plan_identifier: string | null;
  is_sandbox: boolean;
  unsubscribe_detected_at: string | null;
  billing_issues_detected_at: string | null;
  ownership_type: OwnershipType | null;
  verification: VerificationResult;
}

export interface EntitlementInfos {
  all: Record<string, EntitlementInfo>;
  verification: VerificationResult;
}

export interface SubscriptionInfo {
  product_identifier: string;
  purchase_date: string | null;
  original_purchase_date: string | null;
  expires_date: string | null;
  is_active: boolean;
  will_renew: boolean;
  period_type: PeriodType;
  store: Store;
  is_sandbox: boolean;
  unsubscribe_detected_at: string | null;
  billing_issues_detected_at: string | null;
  grace_period_expires_date: string | null;
  refunded_at: string | null;
  auto_resume_date: string | null;
  store_transaction_id: string | null;
  ownership_type: OwnershipType | null;
}

export interface NonSubscriptionTransaction {
  transaction_identifier: string;
  product_identifier: string;
  purchase_date: string;
  store: Store;
  is_sandbox: boolean;
  store_transaction_id: string | null;
}

export interface CustomerInfo {
  request_date: string;
  original_app_user_id: string;
  first_seen: string;
  last_seen: string | null;
  management_url: string | null;
  original_application_version: string | null;
  original_purchase_date: string | null;
  entitlements: EntitlementInfos;
  subscriptions: Record<string, SubscriptionInfo>;
  non_subscription_transactions: NonSubscriptionTransaction[];
  /** Unmodified backend response for additive fields introduced upstream. */
  raw?: unknown;
}

export interface StoreTransaction {
  purchase_token: string;
  product_ids: string[];
  purchase_date: string;
  transaction_id: string | null;
  store: Store;
  price: Price | null;
}

export interface PurchaseResult {
  transaction: StoreTransaction;
  customer_info: CustomerInfo;
}

// -- Plugin envelope types (camelCase) --------------------------------------

export interface ConfigureOptions {
  /** `test_` / `appl_` / `goog_` SDK key. */
  apiKey: string;
  appUserId?: string;
  /** Point at a proxy / staging / mock host (like `Purchases.proxyURL`). */
  proxyUrl?: string;
  /** `"disabled"` | `"informational"` | `"enforced"` (default: informational). */
  entitlementVerificationMode?: "disabled" | "informational" | "enforced";
  /** Base64 Ed25519 root key, for a custom/test signing chain. */
  verificationRootKey?: string;
  /**
   * Allows a Test Store key in an iOS/Android release build. Dangerous:
   * enable only for a controlled test build, never for a production app.
   */
  dangerouslyAllowTestStoreInReleaseBuilds?: boolean;
}

export interface SessionInfo {
  configured: boolean;
  appUserId: string | null;
  isAnonymous: boolean | null;
  /** `"test store"` | `"app store"` | `"play store"`. */
  store: string | null;
}

export interface LoginResult {
  customerInfo: CustomerInfo;
  created: boolean;
}

// -- Commands ---------------------------------------------------------------

const PLUGIN = "plugin:revenuecat";
const NATIVE_TRANSACTION_UPDATED_EVENT = "transaction-updated";
export const NATIVE_SYNC_ERROR_EVENT = "revenuecat:native-sync-error";

export interface NativeSyncError {
  message: string;
}

let nativeTransactionListener: Promise<PluginListener> | undefined;
let nativeLifecycleListeners: Promise<UnlistenFn[]> | undefined;
let nativeSyncInFlight: Promise<void> | undefined;
let nativeSyncRequested = false;
let nativeFullSyncRequested = false;
let nativeRecoveryRequired = false;
let explicitPurchaseRequests = 0;
let nativeRetryTimer: ReturnType<typeof setTimeout> | undefined;
let nativeRetryIncludeAllOwned = false;
let nativeRetryDelayMs = 15_000;

const NATIVE_SYNC_RETRY_DELAYS_MS = [0, 750, 3_000] as const;
const MAX_NATIVE_SYNC_RETRY_DELAY_MS = 15 * 60_000;
const NATIVE_SYNC_FALLBACK_ERROR_MESSAGE = "Native synchronization failed.";

function nonEmptyErrorMessage(error: unknown): string | undefined {
  if (typeof error === "string") {
    return error.trim().length > 0 ? error : undefined;
  }
  if (typeof error !== "object" || error === null) return undefined;

  try {
    const descriptor = Object.getOwnPropertyDescriptor(error, "message");
    if (!descriptor || !("value" in descriptor)) return undefined;
    const message = descriptor.value;
    return typeof message === "string" && message.trim().length > 0
      ? message
      : undefined;
  } catch {
    return undefined;
  }
}

function errorMessage(error: unknown): string {
  const message = nonEmptyErrorMessage(error);
  if (message) return message;

  // JSON scalars are safe to expose. Never stringify unknown records because
  // native error metadata may include credentials, receipts, or user data.
  if (
    typeof error === "boolean" ||
    (typeof error === "number" && Number.isFinite(error))
  ) {
    return JSON.stringify(error);
  }
  return NATIVE_SYNC_FALLBACK_ERROR_MESSAGE;
}

async function ensureNativeTransactionListener(): Promise<void> {
  nativeTransactionListener ??= addPluginListener(
    "revenuecat",
    NATIVE_TRANSACTION_UPDATED_EVENT,
    () => {
      nativeRecoveryRequired = true;
      requestNativeSync(false);
    },
  ).catch((error) => {
    nativeTransactionListener = undefined;
    throw error;
  });
  await nativeTransactionListener;
}

async function installNativeLifecycleListeners(): Promise<UnlistenFn[]> {
  const results = await Promise.allSettled([
    listen(TauriEvent.WINDOW_RESUMED, () => {
      nativeRecoveryRequired = true;
      requestNativeSync(false);
    }),
    listen(TauriEvent.WINDOW_FOCUS, () => {
      nativeRecoveryRequired = true;
      requestNativeSync(false);
    }),
  ]);
  const rejected = results.find(
    (result): result is PromiseRejectedResult => result.status === "rejected",
  );
  if (rejected) {
    await Promise.all(
      results
        .filter(
          (result): result is PromiseFulfilledResult<UnlistenFn> =>
            result.status === "fulfilled",
        )
        .map(({ value: unlisten }) => Promise.resolve(unlisten()).catch(() => undefined)),
    );
    throw rejected.reason;
  }
  return results.map(
    (result) => (result as PromiseFulfilledResult<UnlistenFn>).value,
  );
}

async function ensureNativeLifecycleListeners(): Promise<void> {
  nativeLifecycleListeners ??= installNativeLifecycleListeners().catch((error) => {
    nativeLifecycleListeners = undefined;
    throw error;
  });
  await nativeLifecycleListeners;
}

function wait(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function synchronizeNativePurchases(includeAllOwned: boolean): Promise<void> {
  const command = includeAllOwned
    ? "sync_purchases"
    : "sync_pending_purchases";
  await invoke<CustomerInfo>(`${PLUGIN}|${command}`);
}

async function synchronizeNativePurchasesWithRetry(
  includeAllOwned: boolean,
): Promise<void> {
  let finalError: unknown;
  for (const delayMilliseconds of NATIVE_SYNC_RETRY_DELAYS_MS) {
    if (delayMilliseconds > 0) await wait(delayMilliseconds);
    try {
      await synchronizeNativePurchases(includeAllOwned);
      return;
    } catch (error) {
      finalError = error;
    }
  }
  throw finalError;
}

function scheduleNativeSyncRetry(includeAllOwned: boolean): void {
  nativeRetryIncludeAllOwned ||= includeAllOwned;
  if (nativeRetryTimer) return;

  nativeRetryTimer = setTimeout(() => {
    nativeRetryTimer = undefined;
    const shouldIncludeAllOwned = nativeRetryIncludeAllOwned;
    nativeRetryIncludeAllOwned = false;
    requestNativeSync(shouldIncludeAllOwned);
  }, nativeRetryDelayMs);
  nativeRetryDelayMs = Math.min(
    nativeRetryDelayMs * 2,
    MAX_NATIVE_SYNC_RETRY_DELAY_MS,
  );
}

function recordNativeSyncSuccess(includedAllOwned: boolean): void {
  const queuedSyncStillRequiresRecovery = nativeSyncRequested;
  const stillNeedsFullSync =
    (nativeFullSyncRequested || nativeRetryIncludeAllOwned) &&
    !includedAllOwned;
  if (nativeRetryTimer) clearTimeout(nativeRetryTimer);
  nativeRetryTimer = undefined;
  nativeRetryIncludeAllOwned = false;
  nativeRetryDelayMs = 15_000;
  nativeRecoveryRequired =
    queuedSyncStillRequiresRecovery || stillNeedsFullSync;
  if (stillNeedsFullSync) {
    nativeSyncRequested = true;
    nativeFullSyncRequested = true;
  }
}

async function waitForNativeRecovery(): Promise<void> {
  if (!nativeRecoveryRequired) return;
  if (!nativeSyncInFlight) requestNativeSync(false);
  if (nativeSyncInFlight) await nativeSyncInFlight;
  if (nativeRecoveryRequired) {
    throw new Error(
      "A native purchase still requires synchronization; retry after native sync succeeds.",
    );
  }
}

function requestNativeSync(includeAllOwned: boolean): void {
  nativeSyncRequested = true;
  nativeFullSyncRequested ||= includeAllOwned;
  if (nativeSyncInFlight || explicitPurchaseRequests > 0) return;

  nativeSyncInFlight = (async () => {
    while (nativeSyncRequested) {
      const shouldIncludeAllOwned = nativeFullSyncRequested;
      nativeSyncRequested = false;
      nativeFullSyncRequested = false;
      try {
        await synchronizeNativePurchasesWithRetry(shouldIncludeAllOwned);
        recordNativeSyncSuccess(shouldIncludeAllOwned);
      } catch (error) {
        // Transactions stay unfinished/unacknowledged until a backend post
        // succeeds. Retry transient failures here, then keep an exponentially
        // backed-off timer alive while the webview remains running.
        scheduleNativeSyncRetry(
          shouldIncludeAllOwned || nativeFullSyncRequested,
        );
        nativeSyncRequested = false;
        nativeFullSyncRequested = false;
        await emit<NativeSyncError>(NATIVE_SYNC_ERROR_EVENT, {
          message: errorMessage(error),
        }).catch(() => undefined);
        return;
      }
    }
  })().finally(() => {
    nativeSyncInFlight = undefined;
    if (nativeSyncRequested) requestNativeSync(nativeFullSyncRequested);
  });
}

/** Configure the SDK. On mobile, `appl_`/`goog_` keys wire the native store. */
export async function configure(
  options: ConfigureOptions,
): Promise<SessionInfo> {
  const session = await invoke<SessionInfo>(`${PLUGIN}|configure`, { options });
  if (session.store === "app store" || session.store === "play store") {
    await Promise.all([
      ensureNativeTransactionListener(),
      ensureNativeLifecycleListeners(),
    ]);
    // Play's active-purchase query also recovers acknowledged tokens after a
    // reinstall or lost sent-token cache. StoreKit can stay pending-only here:
    // Transaction.updates + unfinished retain every transaction Rust still
    // needs to post and finish.
    nativeRecoveryRequired = true;
    requestNativeSync(session.store === "play store");
  }
  return session;
}

/** Current session (configured?, app user id, anonymous?, store). */
export async function sessionInfo(): Promise<SessionInfo> {
  return await invoke(`${PLUGIN}|session_info`);
}

/** Offerings with their packages and dashboard paywall config. */
export async function getOfferings(): Promise<Offerings> {
  const offerings = await invoke<OfferingsWire>(`${PLUGIN}|get_offerings`);
  const currentOfferingId = offerings.current_offering_id;
  const current =
    typeof currentOfferingId === "string" &&
    Object.prototype.hasOwnProperty.call(offerings.all, currentOfferingId)
      ? (offerings.all[currentOfferingId] ?? null)
      : null;
  return { ...offerings, current };
}

/** The latest customer info (fetched fresh from the backend). */
export async function getCustomerInfo(): Promise<CustomerInfo> {
  return await invoke(`${PLUGIN}|get_customer_info`);
}

/** Buy a package from the current offering by its identifier. */
export async function purchasePackage(
  packageId: string,
): Promise<PurchaseResult> {
  await waitForNativeRecovery();
  explicitPurchaseRequests += 1;
  try {
    return await invoke(`${PLUGIN}|purchase_package`, { packageId });
  } catch (error) {
    // A native purchase may have succeeded even if the backend post or finish
    // failed. Its StoreKit update is deliberately suppressed as part of the
    // explicit flow, so queue a pending-only recovery before rethrowing.
    nativeRecoveryRequired = true;
    requestNativeSync(false);
    throw error;
  } finally {
    explicitPurchaseRequests -= 1;
    if (explicitPurchaseRequests === 0 && nativeSyncRequested) {
      requestNativeSync(nativeFullSyncRequested);
    }
  }
}

/** Restore purchases and return the refreshed customer info. */
export async function restore(): Promise<CustomerInfo> {
  return await invoke(`${PLUGIN}|restore`);
}

/** Identify the user (alias anonymous purchases); `created` is true for a new id. */
export async function logIn(appUserId: string): Promise<LoginResult> {
  await waitForNativeRecovery();
  return await invoke(`${PLUGIN}|log_in`, { appUserId });
}

/** Sign out to a fresh anonymous id. */
export async function logOut(): Promise<CustomerInfo> {
  await waitForNativeRecovery();
  return await invoke(`${PLUGIN}|log_out`);
}

/** Set the reserved `$email` subscriber attribute. */
export async function setEmail(email: string): Promise<void> {
  await invoke(`${PLUGIN}|set_email`, { email });
}

/**
 * The store's manage/cancel page for the current user, or `null` if none.
 * Open it with `tauri-plugin-opener`; on the App Store / Play Store it lands on
 * the native subscription-management surface.
 */
export async function manageSubscriptions(): Promise<string | null> {
  return await invoke(`${PLUGIN}|manage_subscriptions`);
}

/**
 * Subscribe to customer-info updates — fired on purchase, restore, login, and
 * whenever the SDK refreshes (mirrors RevenueCat's updatedCustomerInfoListener).
 * Returns an unlisten function.
 */
export async function onCustomerInfoUpdated(
  handler: (info: CustomerInfo) => void,
): Promise<UnlistenFn> {
  return await listen<CustomerInfo>(
    "revenuecat:customer-info-updated",
    (event) => handler(event.payload),
  );
}

/** Observe failures from automatic native transaction synchronization. */
export async function onNativeSyncError(
  handler: (error: NativeSyncError) => void,
): Promise<UnlistenFn> {
  return await listen<NativeSyncError>(NATIVE_SYNC_ERROR_EVENT, (event) =>
    handler(event.payload),
  );
}
