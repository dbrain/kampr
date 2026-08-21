package dev.kampr.shared.model

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.wire.PaneInfo

// A join has to hold before it is believed, because a client whose own socket reconnects before
// the node has reaped the dead watch counts itself twice for a moment, and shouting about that is
// shouting about yourself. A departure is held for longer: being briefly wrong about somebody
// still being there is a harmless overstatement that corrects itself, while being briefly wrong
// about somebody arriving is the thing that cries wolf.
const val WATCH_RISE_MS: Long = 2_000
const val WATCH_FALL_MS: Long = 6_000
const val WATCH_NOTICE_MS: Long = 6_000

// One of the watchers is this client — exactly true on a pane it is watching, and one low on a
// pane in a list it is only looking at. Both directions under-count, which is the direction a
// claim about somebody else's terminal has to fail in.
fun othersWatching(pane: PaneInfo?): Int = ((pane?.watchers ?: 1) - 1).coerceAtLeast(0)

fun watchersTag(others: Int): String? = when {
    others <= 0 -> null
    others == 1 -> "also open"
    else -> "also open · $others"
}

// "at least", because a hub relays a whole crowd behind a single watch: the number is a floor and
// stating it as a headcount would be a claim the wire cannot support.
fun watchersPhrase(others: Int): String? = when {
    others <= 0 -> null
    others == 1 -> "also open on another client"
    else -> "also open on at least $others other clients"
}

// The true thing, and only the true thing: another client has this pane on screen. Whether anyone
// over there is typing is not on the wire, and a phone that guessed would be worse than silent.
fun watchersNotice(others: Int): String? = watchersPhrase(others)
    ?.replaceFirstChar { it.uppercase() }
    ?.plus(" — watching, not necessarily typing.")

class WatchPresence(
    private val riseMs: Long = WATCH_RISE_MS,
    private val fallMs: Long = WATCH_FALL_MS,
    private val noticeMs: Long = WATCH_NOTICE_MS,
) {
    var others: Int by mutableStateOf(0)
        private set

    var notice: String? by mutableStateOf(null)
        private set

    private var candidate = 0
    private var since = 0L
    private var noticeUntil = 0L

    fun observe(raw: Int, now: Long) {
        if (raw != candidate) {
            candidate = raw
            since = now
        }
        settle(now)
    }

    fun tick(now: Long) = settle(now)

    fun pending(): Boolean = candidate != others || notice != null

    private fun settle(now: Long) {
        if (candidate != others && now - since >= if (candidate > others) riseMs else fallMs) {
            val arrived = candidate > others
            others = candidate
            if (arrived) {
                notice = watchersNotice(others)
                noticeUntil = now + noticeMs
            } else if (others == 0) {
                notice = null
            }
        }
        if (notice != null && now >= noticeUntil) notice = null
    }
}
