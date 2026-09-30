package dev.kampr.shared.model

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.platform.Prefs
import dev.kampr.shared.wire.PaneInfo

// herdr's `done` is the operator's unread flag — a pane that finished `working`→`idle` while
// nobody was looking at it. Marking one read has to stay a *client-side* fact: the only thing that
// clears herdr's own marker is making the pane the session-focused pane, and `pane.focus`,
// `tab.focus` and `workspace.focus` all do that. That is the operator's press, never a side effect
// of opening a view (rule 3), so nothing here reaches the node.
//
// Read is keyed by the pane's `updatedAt` as well as its id, so the *next* time that pane finishes
// the flag is raised again rather than the pane going quiet for good. A pane with no `updatedAt`
// has nothing to re-arm against and is remembered by id alone — including one that had a stamp
// when it was read: herdr keeps `done` on a pane whose agent has exited, and a node that starts
// after that has no clock for it. That is a node that does not know, not a pane that moved.
private const val NEVER = ""

private const val PAIR = '\u001f'
private const val GAP = '\u001e'

// A device that has watched a thousand panes finish does not need a thousand of them remembered,
// and `Prefs` is one string on every platform that backs it.
private const val KEEP = 256

class SeenDone(private val prefs: Prefs? = null, private val key: String = "seen.done") {
    private var read: Map<String, String> by mutableStateOf(restore())

    // `takeLast`, not `take`: a map keeps insertion order and the newest entry is the one just
    // added, so trimming from the front is the only way to drop the oldest rather than the read
    // the operator just made.
    fun saw(pane: PaneInfo?) {
        if (pane == null || statusOf(pane) != AgentStatus.Done) return
        val grown = read + (pane.id to (pane.updatedAt ?: NEVER))
        read = if (grown.size <= KEEP) grown else grown.entries.toList().takeLast(KEEP).associate { it.key to it.value }
        store()
    }

    internal fun hasRead(pane: PaneInfo): Boolean {
        val at = read[pane.id] ?: return false
        return pane.updatedAt == null || at == pane.updatedAt
    }

    // A pane its own node no longer carries cannot come back under the same id, so remembering it
    // is only growth. A pane whose node is missing too is a hub that has just restarted and whose
    // peers' links are not back yet, and it is the same pane when they are.
    fun keep(herd: Herd) {
        val live = herd.panes.mapTo(mutableSetOf()) { it.id }
        val nodes = herd.nodes.mapTo(mutableSetOf()) { it.id }
        val kept = read.filterKeys { it in live || it.substringBefore('/') !in nodes }
        if (kept.size == read.size) return
        read = kept
        store()
    }

    private fun restore(): Map<String, String> = prefs?.get(key).orEmpty()
        .split(GAP)
        .filter { it.isNotEmpty() }
        .mapNotNull { entry ->
            val at = entry.indexOf(PAIR)
            if (at <= 0) null else entry.substring(0, at) to entry.substring(at + 1)
        }
        .toMap()

    private fun store() {
        prefs?.set(key, read.entries.joinToString(GAP.toString()) { "${it.key}$PAIR${it.value}" }.ifEmpty { null })
    }
}

// The finished panes this device has not read yet — the set a `done` notification stands for.
//
// It is not `store.blocked()`'s twin, because the two empty for different reasons. A question
// leaves the herd when anyone anywhere answers it; a finish leaves *this device's* screen when
// this device reads it, which is a fact only `SeenDone` holds. Reconciling against herdr's own
// `done` alone would leave the notification standing after the operator had read the pane here.
fun Herd.unreadDone(seen: SeenDone): List<PaneInfo> =
    panes.filter { statusOf(it) == AgentStatus.Done && !seen.hasRead(it) }

// One transform, at the one place the UI reads the herd, so the mark, the spoken status, the
// triage list and `paneOrder`'s rank all agree. Demoting only at the render site would leave a
// read pane pinned to the top of the list with no badge on it, which is worse than the bug.
fun Herd.withoutReadDone(seen: SeenDone): Herd {
    if (panes.none { statusOf(it) == AgentStatus.Done && seen.hasRead(it) }) return this
    return copy(
        panes = panes.map {
            if (statusOf(it) == AgentStatus.Done && seen.hasRead(it)) it.copy(agentStatus = "idle") else it
        },
    )
}
