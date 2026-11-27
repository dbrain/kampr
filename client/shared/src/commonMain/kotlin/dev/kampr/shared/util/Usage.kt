package dev.kampr.shared.util

import dev.kampr.shared.wire.Billing
import dev.kampr.shared.wire.Limits
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

// What the billing line says of a session. `provider` is who the session runs on, `key` is which
// key pays, and `rateLimited` is the provider saying it is throttling: each is its own fact and
// only the ones present are said.
fun billingWord(billing: Billing): String? {
    val who = payer(billing) ?: return null
    return if (billing.rateLimited) "$who, rate-limited" else "billed by $who"
}

private fun payer(billing: Billing): String? = billing.provider ?: billing.key?.let { "key $it" }

// `usage` is an open value the provider counts in its own units, so it is said only where it is a
// single readable number or word — an object the client did not ask for is not one it guesses at.
fun usageWord(limits: Limits): String? {
    val u = limits.usage as? JsonPrimitive ?: return null
    return when {
        u.isString -> u.content
        u.contentOrNull?.toLongOrNull() != null -> "${u.contentOrNull}%"
        else -> null
    }
}

// How long until an instant, coarse enough to read at a glance and fine enough to move while it
// is watched: seconds only inside the last hour.
fun remainingSpan(millis: Double): String {
    val seconds = (millis / 1000.0).toLong().coerceAtLeast(0)
    return when {
        seconds < 60 -> "${seconds}s"
        seconds < 3600 -> "${seconds / 60}m ${seconds % 60}s"
        seconds < 86_400 -> "${seconds / 3600}h ${seconds % 3600 / 60}m"
        else -> "${seconds / 86_400}d ${seconds % 86_400 / 3600}h"
    }
}

// A moment the source named, as the reader's own clock face and, while it is still ahead, how long
// until it. An ISO stamp on a phone is a number nobody can subtract from the time in their head.
fun dueWord(atMillis: Double, nowMillis: Double, offsetOf: (Double) -> Double = ::localOffsetMillis): String {
    val face = clockFace(atMillis, nowMillis, offsetOf(atMillis))
    return if (atMillis > nowMillis) "in ${remainingSpan(atMillis - nowMillis)} ($face)" else face
}

// The same moment said as the end of a wait: the time first, because "held until" asks for one.
fun untilWord(atMillis: Double, nowMillis: Double, offsetOf: (Double) -> Double = ::localOffsetMillis): String {
    val face = clockFace(atMillis, nowMillis, offsetOf(atMillis))
    return if (atMillis > nowMillis) "$face (in ${remainingSpan(atMillis - nowMillis)})" else face
}

fun limitsWord(limits: Limits, nowMillis: Double, offsetOf: (Double) -> Double = ::localOffsetMillis): String? {
    val parts = buildList {
        limits.plan?.let { add(it) }
        usageWord(limits)?.let { add(it) }
        limits.resetsAt?.let { at -> add("resets " + (parseIsoMillis(at)?.let { dueWord(it, nowMillis, offsetOf) } ?: at)) }
    }
    return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
}

// The header's chip: how much is used, when it comes back and who pays, in as few words as say it.
// A clock face rather than a countdown, because the header is not drawn again every second.
fun usageChip(limits: Limits?, billing: Billing?, nowMillis: Double, offsetOf: (Double) -> Double = ::localOffsetMillis): String? {
    val parts = buildList {
        limits?.let(::usageWord)?.let { add(it) }
        limits?.resetsAt?.let(::parseIsoMillis)?.let { add("resets " + clockFace(it, nowMillis, offsetOf(it))) }
        billing?.let(::payer)?.let { add(if (billing.rateLimited) "$it, rate-limited" else it) }
    }
    return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
}
