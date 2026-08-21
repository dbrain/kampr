package dev.kampr.terminal.review

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

private const val NO_WORD = -1

// ADR 0010's missing piece. The reader owns this cursor; a `grid.patch` never moves it and never
// speaks over it. What a repaint underneath is allowed to do is set a flag, which becomes one
// quiet notice and a prefix on the next thing the reader asks to hear.
@Stable
class ReviewState {
    var active by mutableStateOf(false)
        private set

    var row by mutableIntStateOf(0)
        private set

    var word by mutableIntStateOf(NO_WORD)
        private set

    // What the live region says. Reader-driven only: nothing on this surface writes it.
    var utterance by mutableStateOf("")
        private set

    // Fires once per change under the reader, and only while they are parked on it.
    var notice by mutableStateOf("")
        private set

    var touched by mutableStateOf(false)
        private set

    var lost by mutableStateOf(false)
        private set

    // The same line read twice is the same string, and a live region only speaks on a change. An
    // unspoken no-break space rides along so "read it again" is never silence.
    var reads by mutableIntStateOf(0)
        private set

    val spoken: String get() = if (reads % 2 == 0) utterance else "$utterance "

    private var anchor: ReviewAnchor? = null
    private var held = ""

    fun enter(surface: ReviewSurface) {
        active = true
        touched = false
        lost = false
        word = NO_WORD
        anchor = anchorAt(surface, surface.cursorIndex)
        read(surface, depth = true)
    }

    fun leave() {
        active = false
        anchor = null
        utterance = ""
        notice = ""
        touched = false
        lost = false
        word = NO_WORD
    }

    // Called on every revision. It resolves the anchor against the surface as it is now — which is
    // how a discarded row is noticed — and never changes where the reader is pointing.
    fun sync(surface: ReviewSurface) {
        if (!active) return
        row = resolve(surface)
        val now = surface.rowAt(row)
        if (now == held) return
        held = now
        if (!touched) {
            touched = true
            notice = "The pane wrote to the row you are reading."
        }
    }

    fun step(surface: ReviewSurface, move: ReviewMove) {
        if (!active) return
        row = resolve(surface)
        when (move) {
            ReviewMove.Reread -> read(surface, depth = false)
            ReviewMove.Now -> {
                word = NO_WORD
                anchor = anchorAt(surface, surface.cursorIndex)
                read(surface, depth = true)
            }
            ReviewMove.PreviousLine -> {
                word = NO_WORD
                moveTo(surface, row - 1)
            }
            ReviewMove.NextLine -> {
                word = NO_WORD
                moveTo(surface, row + 1)
            }
            ReviewMove.NextWord -> {
                val words = reviewWords(surface.rowAt(row))
                if (word + 1 < words.size) {
                    word++
                    read(surface, depth = false)
                } else {
                    word = NO_WORD
                    if (moveTo(surface, row + 1)) firstWord(surface)
                }
            }
            ReviewMove.PreviousWord -> {
                if (word > 0) {
                    word--
                    read(surface, depth = false)
                } else {
                    word = NO_WORD
                    if (moveTo(surface, row - 1)) lastWord(surface)
                }
            }
        }
    }

    private fun firstWord(surface: ReviewSurface) {
        if (reviewWords(surface.rowAt(row)).isEmpty()) return
        word = 0
        read(surface, depth = false)
    }

    private fun lastWord(surface: ReviewSurface) {
        val words = reviewWords(surface.rowAt(row))
        if (words.isEmpty()) return
        word = words.lastIndex
        read(surface, depth = false)
    }

    // An edge is an answer, not a refusal: the reader hears why the record stops rather than
    // pressing a control that does nothing.
    private fun moveTo(surface: ReviewSurface, index: Int): Boolean {
        if (index < 0) {
            say(historyEdgeSpoken(surface))
            return false
        }
        if (index >= surface.total) {
            say("Bottom of the pane. This is the newest row.")
            return false
        }
        anchor = anchorAt(surface, index)
        read(surface, depth = false)
        return true
    }

    private fun read(surface: ReviewSurface, depth: Boolean) {
        row = resolve(surface)
        val text = surface.rowAt(row)
        held = text
        val body = if (word == NO_WORD) {
            val where = if (depth) "row ${row + 1} of ${surface.total}" else "row ${row + 1}"
            "$where. ${text.ifBlank { "blank line" }}"
        } else {
            val words = reviewWords(text)
            val at = word.coerceIn(0, (words.size - 1).coerceAtLeast(0))
            if (words.isEmpty()) "row ${row + 1}. blank line"
            else "${words[at]}, word ${at + 1} of ${words.size}"
        }
        say(prefix() + body)
    }

    private fun prefix(): String = when {
        lost -> "Those rows were discarded. "
        touched -> "Changed. "
        else -> ""
    }

    private fun say(text: String) {
        utterance = text
        reads++
        touched = false
        lost = false
        notice = ""
    }

    private fun anchorAt(surface: ReviewSurface, index: Int): ReviewAnchor {
        val bounded = index.coerceIn(0, (surface.total - 1).coerceAtLeast(0))
        return if (bounded >= surface.historyRows) {
            ReviewAnchor.Live(bounded - surface.historyRows)
        } else {
            ReviewAnchor.History(surface.fromTop + bounded)
        }
    }

    private fun resolve(surface: ReviewSurface): Int {
        val last = (surface.total - 1).coerceAtLeast(0)
        return when (val held = anchor) {
            null -> 0
            is ReviewAnchor.Live -> (surface.historyRows + held.row).coerceIn(0, last)
            is ReviewAnchor.History -> {
                if (held.absolute < surface.fromTop) lost = true
                (held.absolute - surface.fromTop).coerceIn(0, last)
            }
        }
    }
}
