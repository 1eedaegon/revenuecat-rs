package app.tauri.revenuecat

internal data class ProductQuery(
    val productId: String,
    val basePlanId: String?,
)

internal fun normalizeProductQueries(
    ids: List<String>,
    queries: List<ProductQuery>,
): List<ProductQuery> = if (queries.isNotEmpty()) {
    queries
} else {
    ids.distinct().map { ProductQuery(it, null) }
}
