package dev.kampr.shared.model

import androidx.compose.runtime.Immutable

// Triage cards the operator put away, per pane, for as long as that pane stays blocked on the same
// question. Unknown is a match either way: the node republishes a question a beat after a blocked
// edge or a reconnect, and the card must not flash back in that gap.
@Immutable
data class Dismissed(private val held: Map<String, String> = emptyMap()) {
    fun hides(item: TriageItem): Boolean {
        val kept = held[item.pane.id] ?: return false
        val now = keyOf(item)
        return kept == UNKNOWN || now == UNKNOWN || kept == now
    }

    fun with(item: TriageItem): Dismissed = copy(held = held + (item.pane.id to keyOf(item)))

    // Only on a herd that is current: a stale one after a dropped socket says nothing about which
    // panes are still blocked.
    fun prune(herd: Herd): Dismissed {
        if (!herd.known || herd.stale) return this
        val blocked = herd.panes.filter { statusOf(it) == AgentStatus.Blocked }.map { it.id }.toSet()
        return copy(held = held.filterKeys { it in blocked })
    }

    fun encode(): String = held.entries.joinToString("\n") { (pane, key) -> "$pane\t$key" }

    companion object {
        private const val UNKNOWN = ""

        private fun keyOf(item: TriageItem): String = item.questionId ?: item.question?.replace(Regex("\\s+"), " ") ?: UNKNOWN

        // A pane id never holds a tab or a newline; a question's text is flattened so it cannot.
        fun decode(text: String?): Dismissed = Dismissed(
            text.orEmpty().lines().mapNotNull { line ->
                line.split('\t', limit = 2).takeIf { it.size == 2 }?.let { it[0] to it[1] }
            }.toMap(),
        )
    }
}
