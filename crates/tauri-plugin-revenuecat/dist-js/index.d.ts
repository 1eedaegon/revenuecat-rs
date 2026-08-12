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
export type ProductType = "subscription" | "consumable" | "non_consumable" | "unknown";
export type PeriodType = "normal" | "intro" | "trial" | "prepaid" | "unknown";
export type Store = "app_store" | "mac_app_store" | "play_store" | "stripe" | "promotional" | "amazon" | "rc_billing" | "external" | "paddle" | "test_store" | "unknown";
export type OwnershipType = "PURCHASED" | "FAMILY_SHARED";
export type VerificationResult = "NotRequested" | "Verified" | "VerifiedOnDevice" | "Failed";
/** Well-known `$rc_*` packages get a named type; anything else is `{ Custom }`. */
export type PackageType = "Lifetime" | "Annual" | "SixMonth" | "ThreeMonth" | "TwoMonth" | "Monthly" | "Weekly" | {
    Custom: string;
};
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
    current: Offering | null;
    all: Record<string, Offering>;
}
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
    colors: {
        light: PaywallColors;
        dark: PaywallColors | null;
    };
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
/** Configure the SDK. On mobile, `appl_`/`goog_` keys wire the native store. */
export declare function configure(options: ConfigureOptions): Promise<SessionInfo>;
/** Current session (configured?, app user id, anonymous?, store). */
export declare function sessionInfo(): Promise<SessionInfo>;
/** Offerings with their packages and dashboard paywall config. */
export declare function getOfferings(): Promise<Offerings>;
/** The latest customer info (fetched fresh from the backend). */
export declare function getCustomerInfo(): Promise<CustomerInfo>;
/** Buy a package from the current offering by its identifier. */
export declare function purchasePackage(packageId: string): Promise<PurchaseResult>;
/** Restore purchases and return the refreshed customer info. */
export declare function restore(): Promise<CustomerInfo>;
/** Identify the user (alias anonymous purchases); `created` is true for a new id. */
export declare function logIn(appUserId: string): Promise<LoginResult>;
/** Sign out to a fresh anonymous id. */
export declare function logOut(): Promise<CustomerInfo>;
/** Set the reserved `$email` subscriber attribute. */
export declare function setEmail(email: string): Promise<void>;
