/**
 * Typed JS/TS bindings for `tauri-plugin-revenuecat`.
 *
 * Register the plugin in Rust (`.plugin(tauri_plugin_revenuecat::init())`),
 * then drive RevenueCat entirely from the webview:
 *
 * ```ts
 * import { configure, getOfferings, purchasePackage } from "tauri-plugin-revenuecat-api";
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
import { invoke } from "@tauri-apps/api/core";
// -- Commands ---------------------------------------------------------------
const PLUGIN = "plugin:revenuecat";
/** Configure the SDK. On mobile, `appl_`/`goog_` keys wire the native store. */
export async function configure(options) {
    return await invoke(`${PLUGIN}|configure`, { options });
}
/** Current session (configured?, app user id, anonymous?, store). */
export async function sessionInfo() {
    return await invoke(`${PLUGIN}|session_info`);
}
/** Offerings with their packages and dashboard paywall config. */
export async function getOfferings() {
    return await invoke(`${PLUGIN}|get_offerings`);
}
/** The latest customer info (fetched fresh from the backend). */
export async function getCustomerInfo() {
    return await invoke(`${PLUGIN}|get_customer_info`);
}
/** Buy a package from the current offering by its identifier. */
export async function purchasePackage(packageId) {
    return await invoke(`${PLUGIN}|purchase_package`, { packageId });
}
/** Restore purchases and return the refreshed customer info. */
export async function restore() {
    return await invoke(`${PLUGIN}|restore`);
}
/** Identify the user (alias anonymous purchases); `created` is true for a new id. */
export async function logIn(appUserId) {
    return await invoke(`${PLUGIN}|log_in`, { appUserId });
}
/** Sign out to a fresh anonymous id. */
export async function logOut() {
    return await invoke(`${PLUGIN}|log_out`);
}
/** Set the reserved `$email` subscriber attribute. */
export async function setEmail(email) {
    await invoke(`${PLUGIN}|set_email`, { email });
}
