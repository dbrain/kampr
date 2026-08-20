package dev.kampr.shared.util

fun parseIsoMillis(text: String?): Double? {
    if (text.isNullOrBlank()) return null
    val year = text.substring(0, 4).toIntOrNull() ?: return null
    val month = text.substring(5, 7).toIntOrNull() ?: return null
    val day = text.substring(8, 10).toIntOrNull() ?: return null
    val hour = text.substring(11, 13).toIntOrNull() ?: return null
    val minute = text.substring(14, 16).toIntOrNull() ?: return null
    val second = text.substring(17, 19).toIntOrNull() ?: 0
    return (daysFromCivil(year, month, day) * 86_400.0 + hour * 3_600.0 + minute * 60.0 + second) * 1000.0
}

private fun daysFromCivil(year: Int, month: Int, day: Int): Long {
    val y = if (month <= 2) year - 1 else year
    val era = (if (y >= 0) y else y - 399) / 400
    val yoe = y - era * 400
    val mp = (month + 9) % 12
    val doy = (153 * mp + 2) / 5 + day - 1
    val doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
    return era.toLong() * 146_097L + doe - 719_468L
}

fun relativeTime(updatedAt: String?, nowMillis: Double): String {
    val at = parseIsoMillis(updatedAt) ?: return "—"
    val seconds = ((nowMillis - at) / 1000.0).toLong()
    return when {
        seconds < 45 -> "now"
        seconds < 3600 -> "${seconds / 60}m"
        seconds < 86_400 -> "${seconds / 3600}h"
        else -> "${seconds / 86_400}d"
    }
}

fun formatLatency(ms: Double?): String {
    if (ms == null) return "—"
    val tenths = (ms * 10).toLong()
    if (ms >= 10 || tenths % 10L == 0L) return "${ms.toLong()} ms"
    return "${tenths / 10}.${tenths % 10} ms"
}
