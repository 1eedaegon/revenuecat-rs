package app.tauri.revenuecat

internal data class SubscriptionOptionRequest(
    val basePlanId: String? = null,
    val offerId: String? = null,
    val offerToken: String? = null,
)

internal data class SubscriptionOptionDescriptor(
    val basePlanId: String,
    val offerId: String?,
    val offerToken: String,
    val tags: Set<String>,
    val pricingPhases: List<SubscriptionPricingPhase>,
) {
    val isBasePlan: Boolean
        get() = offerId == null
}

internal data class SubscriptionPricingPhase(
    val priceAmountMicros: Long,
    val billingPeriod: String,
)

internal fun selectSubscriptionOption(
    options: List<SubscriptionOptionDescriptor>,
    request: SubscriptionOptionRequest,
): SubscriptionOptionDescriptor? {
    request.offerToken?.takeIf(String::isNotBlank)?.let { requestedToken ->
        return options.firstOrNull { it.offerToken == requestedToken }
    }

    val requestedBasePlanId = request.basePlanId?.takeIf(String::isNotBlank)
    val requestedOfferId = request.offerId?.takeIf(String::isNotBlank)
    val matchingOptions = options.filter { option ->
        (requestedBasePlanId == null || option.basePlanId == requestedBasePlanId) &&
            (requestedOfferId == null || option.offerId == requestedOfferId)
    }
    if ((requestedBasePlanId != null || requestedOfferId != null) && matchingOptions.isEmpty()) {
        return null
    }
    if (requestedOfferId != null) {
        return matchingOptions.firstOrNull()
    }

    return selectDefaultSubscriptionOption(matchingOptions)
}

private fun selectDefaultSubscriptionOption(
    options: List<SubscriptionOptionDescriptor>,
): SubscriptionOptionDescriptor? {
    val basePlan = options.firstOrNull(SubscriptionOptionDescriptor::isBasePlan) ?: return null
    val eligibleOffers = options
        .asSequence()
        .filterNot(SubscriptionOptionDescriptor::isBasePlan)
        .filterNot { option -> option.tags.any(EXCLUDED_OFFER_TAGS::contains) }
        .toList()

    return longestFreeTrial(eligibleOffers)
        ?: cheapestIntroductoryOffer(eligibleOffers)
        ?: basePlan
}

private fun longestFreeTrial(
    options: List<SubscriptionOptionDescriptor>,
): SubscriptionOptionDescriptor? = options
    .mapNotNull { option ->
        option.introductoryPhases()
            .firstOrNull { it.priceAmountMicros == 0L }
            ?.let { option to billingPeriodToDays(it.billingPeriod) }
    }
    .maxByOrNull { (_, durationDays) -> durationDays }
    ?.first

private fun cheapestIntroductoryOffer(
    options: List<SubscriptionOptionDescriptor>,
): SubscriptionOptionDescriptor? = options
    .mapNotNull { option ->
        option.introductoryPhases()
            .firstOrNull { it.priceAmountMicros > 0L }
            ?.let { option to it.priceAmountMicros }
    }
    .minByOrNull { (_, priceAmountMicros) -> priceAmountMicros }
    ?.first

private fun SubscriptionOptionDescriptor.introductoryPhases(): List<SubscriptionPricingPhase> =
    pricingPhases.dropLast(1)

private fun billingPeriodToDays(period: String): Int {
    val match = BILLING_PERIOD_PATTERN.matchEntire(period) ?: return 0
    val years = match.groups[1]?.value?.toIntOrNull() ?: 0
    val months = match.groups[2]?.value?.toIntOrNull() ?: 0
    val weeks = match.groups[3]?.value?.toIntOrNull() ?: 0
    val days = match.groups[4]?.value?.toIntOrNull() ?: 0
    return years * DAYS_PER_YEAR + months * DAYS_PER_MONTH + weeks * DAYS_PER_WEEK + days
}

private const val DAYS_PER_WEEK = 7
private const val DAYS_PER_MONTH = 30
private const val DAYS_PER_YEAR = 365
private val BILLING_PERIOD_PATTERN = Regex("P(?:(\\d+)Y)?(?:(\\d+)M)?(?:(\\d+)W)?(?:(\\d+)D)?")
private val EXCLUDED_OFFER_TAGS = setOf("rc-ignore-offer", "rc-customer-center")
