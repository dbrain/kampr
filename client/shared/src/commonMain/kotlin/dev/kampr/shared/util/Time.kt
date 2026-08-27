package dev.kampr.shared.util

fun parseIsoMillis(text: String?): Double? {
    if (text.isNullOrBlank()) return null
    val year = text.substring(0, 4).toIntOrNull() ?: return null
    val month = text.substring(5, 7).toIntOrNull() ?: return null
    val day = text.substring(8, 10).toIntOrNull() ?: return null
    val hour = text.substring(11, 13).toIntOrNull() ?: return null
    val minute = text.substring(14, 16).toIntOrNull() ?: return null
    val second = text.substring(17, 19).toIntOrNull() ?: 0
    val civil = (daysFromCivil(year, month, day) * 86_400.0 + hour * 3_600.0 + minute * 60.0 + second) * 1000.0
    return civil - (isoZoneMillis(text) ?: 0.0)
}

// Whether the stamp names the offset it is in. Every harness Kampr reads writes UTC and says so
// with a `Z` — claude 2.x, codex and agy all do (#285) — which is what makes a clock face here
// honest rather than a guess. One that says nothing is read as UTC for an age, which is the most
// it can be trusted for, and never drawn as a time of day.
fun isoIsZoned(text: String?): Boolean = isoZoneMillis(text) != null

private fun isoZoneMillis(text: String?): Double? {
    val stamp = text?.trim() ?: return null
    if (stamp.length < 20) return null
    if (stamp.endsWith("Z") || stamp.endsWith("z")) return 0.0
    val sign = stamp.lastIndexOf('+').takeIf { it >= 19 }
        ?: stamp.lastIndexOf('-').takeIf { it >= 19 }
        ?: return null
    val zone = stamp.substring(sign + 1).replace(":", "")
    if (zone.length != 4) return null
    val hours = zone.substring(0, 2).toIntOrNull() ?: return null
    val minutes = zone.substring(2, 4).toIntOrNull() ?: return null
    val magnitude = (hours * 3_600.0 + minutes * 60.0) * 1000.0
    return if (stamp[sign] == '-') -magnitude else magnitude
}

private fun floorDiv(a: Long, b: Long): Long {
    val q = a / b
    return if (a % b != 0L && (a xor b) < 0L) q - 1 else q
}

private fun floorMod(a: Long, b: Long): Long = a - floorDiv(a, b) * b

private fun daysFromCivil(year: Int, month: Int, day: Int): Long {
    val y = if (month <= 2) year - 1 else year
    val era = (if (y >= 0) y else y - 399) / 400
    val yoe = y - era * 400
    val mp = (month + 9) % 12
    val doy = (153 * mp + 2) / 5 + day - 1
    val doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
    return era.toLong() * 146_097L + doe - 719_468L
}

private fun civilFromDays(days: Long): Triple<Int, Int, Int> {
    val z = days + 719_468L
    val era = floorDiv(z, 146_097L)
    val doe = z - era * 146_097L
    val yoe = (doe - doe / 1460L + doe / 36_524L - doe / 146_096L) / 365L
    val doy = doe - (365L * yoe + yoe / 4L - yoe / 100L)
    val mp = (5L * doy + 2L) / 153L
    val day = doy - (153L * mp + 2L) / 5L + 1L
    val month = if (mp < 10L) mp + 3L else mp - 9L
    val year = yoe + era * 400L + if (month <= 2L) 1L else 0L
    return Triple(year.toInt(), month.toInt(), day.toInt())
}

fun relativeTime(updatedAt: String?, nowMillis: Double): String {
    val at = parseIsoMillis(updatedAt) ?: return "—"
    return elapsed(((nowMillis - at) / 1000.0).toLong())
}

fun relativeSeconds(epochSeconds: Long, nowMillis: Double): String =
    elapsed((nowMillis / 1000.0).toLong() - epochSeconds)

private fun elapsed(seconds: Long): String = when {
    seconds < 45 -> "now"
    seconds < 3600 -> "${seconds / 60}m"
    seconds < 86_400 -> "${seconds / 3600}h"
    else -> "${seconds / 86_400}d"
}

// The reader's own clock face, in the reader's own zone. An age answers "how long ago" and
// nothing else: it is stale the moment it is painted, it cannot be compared against anything the
// operator saw elsewhere, and "now" beside a message written an hour ago is a lie a stopped
// ticker tells. An instant is none of those things — and because `at` is an absolute instant, a
// device whose own clock is minutes out still draws the right face for its zone.
//
// `offsetMillis` is passed rather than looked up so this stays a pure function of the instant and
// the zone; [localOffsetMillis] is what supplies it, per instant, so a transcript that crosses a
// daylight-saving boundary reads correctly on both sides of it.
fun clockFace(atMillis: Double, nowMillis: Double, offsetMillis: Double): String {
    val local = ((atMillis + offsetMillis) / 1000.0).toLong()
    val day = floorDiv(local, 86_400L)
    val seconds = floorMod(local, 86_400L)
    val clock = "${pad2(seconds / 3600L)}:${pad2(seconds % 3600L / 60L)}"
    val today = floorDiv(((nowMillis + offsetMillis) / 1000.0).toLong(), 86_400L)
    if (day == today) return clock
    if (day in (today - 6L) until today) return "${DAYS[floorMod(day + 3L, 7L).toInt()]} $clock"
    val (year, month, date) = civilFromDays(day)
    val (thisYear, _, _) = civilFromDays(today)
    val stamp = "$date ${MONTHS[month - 1]}"
    return if (year == thisYear) "$stamp $clock" else "$stamp $year $clock"
}

private fun pad2(value: Long): String = if (value < 10L) "0$value" else "$value"

fun formatLatency(ms: Double?): String {
    if (ms == null) return "—"
    val tenths = (ms * 10).toLong()
    if (ms >= 10 || tenths % 10L == 0L) return "${ms.toLong()} ms"
    return "${tenths / 10}.${tenths % 10} ms"
}

private val DAYS = listOf("Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun")

private val MONTHS = listOf("Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec")

// RFC 9110's preferred form: "Thu, 20 Aug 2026 23:21:08 GMT". Every HTTP response carries one, and
// it is the only reading of the node's clock a client gets without a new field on `hello` — which
// matters because a snooze is computed here and filtered there, and a pane's age is stamped there
// and rendered here. A phone two minutes fast reported every pane as "now".
fun parseHttpDateMillis(text: String?): Double? {
    val parts = text?.trim()?.split(' ')?.filter { it.isNotEmpty() } ?: return null
    if (parts.size < 5) return null
    val day = parts[1].toIntOrNull() ?: return null
    val month = MONTHS.indexOf(parts[2]).takeIf { it >= 0 }?.plus(1) ?: return null
    val year = parts[3].toIntOrNull() ?: return null
    val hms = parts[4].split(':').mapNotNull { it.toIntOrNull() }
    if (hms.size != 3) return null
    return (daysFromCivil(year, month, day) * 86_400.0 + hms[0] * 3_600.0 + hms[1] * 60.0 + hms[2]) * 1000.0
}
