package app.tauri.revenuecat

import kotlin.test.Test
import kotlin.test.assertEquals

class ProductQueryTest {
    @Test
    fun `legacy ids remain supported and de-duplicated`() {
        val queries = normalizeProductQueries(
            ids = listOf("monthly", "monthly", "lifetime"),
            queries = emptyList(),
        )

        assertEquals(
            listOf(ProductQuery("monthly", null), ProductQuery("lifetime", null)),
            queries,
        )
    }

    @Test
    fun `plan-aware queries preserve order and duplicate product ids`() {
        val requestedQueries = listOf(
            ProductQuery("subscription", "monthly"),
            ProductQuery("subscription", "annual"),
        )

        val queries = normalizeProductQueries(
            ids = listOf("legacy-is-ignored"),
            queries = requestedQueries,
        )

        assertEquals(requestedQueries, queries)
    }
}
