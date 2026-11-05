package app.tauri.revenuecat

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class SubscriptionOptionSelectorTest {
    private val basePlan = option(
        basePlanId = "monthly",
        offerId = null,
        offerToken = "base-token",
        phases = listOf(phase(9_990_000, "P1M")),
    )

    @Test
    fun `selects requested offer token exactly`() {
        val trial = option(
            basePlanId = "monthly",
            offerId = "trial",
            offerToken = "trial-token",
            phases = listOf(phase(0, "P7D"), phase(9_990_000, "P1M")),
        )

        val selected = selectSubscriptionOption(
            options = listOf(basePlan, trial),
            request = SubscriptionOptionRequest(offerToken = "trial-token"),
        )

        assertEquals(trial, selected)
    }

    @Test
    fun `selects requested base plan and offer id exactly`() {
        val trial = option(
            basePlanId = "monthly",
            offerId = "trial",
            offerToken = "trial-token",
            phases = listOf(phase(0, "P7D"), phase(9_990_000, "P1M")),
        )

        val selected = selectSubscriptionOption(
            options = listOf(basePlan, trial),
            request = SubscriptionOptionRequest(
                basePlanId = "monthly",
                offerId = "trial",
            ),
        )

        assertEquals(trial, selected)
    }

    @Test
    fun `selects default offer within requested base plan`() {
        val shortTrial = option(
            basePlanId = "monthly",
            offerId = "short-trial",
            offerToken = "short-token",
            phases = listOf(phase(0, "P7D"), phase(9_990_000, "P1M")),
        )
        val longTrial = option(
            basePlanId = "monthly",
            offerId = "long-trial",
            offerToken = "long-token",
            phases = listOf(phase(0, "P1M"), phase(9_990_000, "P1M")),
        )
        val annual = option(
            basePlanId = "annual",
            offerId = null,
            offerToken = "annual-token",
            phases = listOf(phase(99_990_000, "P1Y")),
        )

        val selected = selectSubscriptionOption(
            options = listOf(basePlan, shortTrial, longTrial, annual),
            request = SubscriptionOptionRequest(basePlanId = "monthly"),
        )

        assertEquals(longTrial, selected)
    }

    @Test
    fun `selects the best default offer across all base plans when no plan is requested`() {
        val monthlyTrial = option(
            basePlanId = "monthly",
            offerId = "monthly-trial",
            offerToken = "monthly-trial-token",
            phases = listOf(phase(0, "P7D"), phase(9_990_000, "P1M")),
        )
        val annualBasePlan = option(
            basePlanId = "annual",
            offerId = null,
            offerToken = "annual-base-token",
            phases = listOf(phase(99_990_000, "P1Y")),
        )
        val annualTrial = option(
            basePlanId = "annual",
            offerId = "annual-trial",
            offerToken = "annual-trial-token",
            phases = listOf(phase(0, "P1M"), phase(99_990_000, "P1Y")),
        )

        val selected = selectSubscriptionOption(
            options = listOf(basePlan, monthlyTrial, annualBasePlan, annualTrial),
            request = SubscriptionOptionRequest(),
        )

        assertEquals(annualTrial, selected)
    }

    @Test
    fun `ignores RevenueCat excluded offers`() {
        val ignoredTrial = option(
            basePlanId = "monthly",
            offerId = "ignored",
            offerToken = "ignored-token",
            tags = setOf("rc-ignore-offer"),
            phases = listOf(phase(0, "P1Y"), phase(9_990_000, "P1M")),
        )
        val eligibleTrial = option(
            basePlanId = "monthly",
            offerId = "eligible",
            offerToken = "eligible-token",
            phases = listOf(phase(0, "P7D"), phase(9_990_000, "P1M")),
        )

        val selected = selectSubscriptionOption(
            options = listOf(basePlan, ignoredTrial, eligibleTrial),
            request = SubscriptionOptionRequest(),
        )

        assertEquals(eligibleTrial, selected)
    }

    @Test
    fun `selects cheapest introductory phase when no free trial exists`() {
        val expensiveIntro = option(
            basePlanId = "monthly",
            offerId = "expensive",
            offerToken = "expensive-token",
            phases = listOf(phase(4_990_000, "P1M"), phase(9_990_000, "P1M")),
        )
        val cheapIntro = option(
            basePlanId = "monthly",
            offerId = "cheap",
            offerToken = "cheap-token",
            phases = listOf(phase(990_000, "P1M"), phase(9_990_000, "P1M")),
        )

        val selected = selectSubscriptionOption(
            options = listOf(basePlan, expensiveIntro, cheapIntro),
            request = SubscriptionOptionRequest(),
        )

        assertEquals(cheapIntro, selected)
    }

    @Test
    fun `falls back to bare base plan when no eligible offer exists`() {
        val selected = selectSubscriptionOption(
            options = listOf(basePlan),
            request = SubscriptionOptionRequest(),
        )

        assertEquals(basePlan, selected)
    }

    @Test
    fun `returns null when explicit option does not exist`() {
        val selected = selectSubscriptionOption(
            options = listOf(basePlan),
            request = SubscriptionOptionRequest(basePlanId = "missing"),
        )

        assertNull(selected)
    }

    private fun option(
        basePlanId: String,
        offerId: String?,
        offerToken: String,
        tags: Set<String> = emptySet(),
        phases: List<SubscriptionPricingPhase>,
    ) = SubscriptionOptionDescriptor(
        basePlanId = basePlanId,
        offerId = offerId,
        offerToken = offerToken,
        tags = tags,
        pricingPhases = phases,
    )

    private fun phase(priceAmountMicros: Long, billingPeriod: String) = SubscriptionPricingPhase(
        priceAmountMicros = priceAmountMicros,
        billingPeriod = billingPeriod,
    )
}
