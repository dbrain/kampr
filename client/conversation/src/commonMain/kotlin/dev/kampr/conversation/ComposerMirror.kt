package dev.kampr.conversation

import dev.kampr.shared.model.DeskLine
import dev.kampr.shared.wire.EditKeys
import kotlin.time.TimeSource

// The reply box and the pane's own box, kept as one line.
//
// `line` is what the pane's box holds as far as this box knows — the node's last reading of it,
// moved on by every key this box has typed since — and `caret` is where the pane's caret is in it.
// The box is *in step* while it holds exactly `line`: only then is an edit typed into the pane, and
// only then is a line the desk typed taken up into the box. A box the operator has written in out
// of step is theirs, and keeps its words until it is sent.
//
// Every key here follows a press in the box, and taking up the desk's line is a read (rule 3).
class ComposerMirror {
    private var line: String? = null
    private var caret = 0
    private var keys: EditKeys? = null
    private var quietUntil = 0L
    private var unheard: DeskLine? = null

    // What the box should now hold, or null to leave it alone.
    fun heard(desk: DeskLine?, box: String, now: Long = elapsed()): String? {
        keys = desk?.keys
        if (desk == null || desk.keys == null) return null
        if (now < quietUntil && desk.text != line) {
            unheard = desk
            return null
        }
        unheard = null
        val inStep = box == (line ?: "")
        line = desk.text
        caret = (desk.caret ?: desk.text.length).coerceIn(0, desk.text.length)
        return desk.text.takeIf { inStep && it != box }
    }

    // The desk's last line, once the box's own keys have had time to come back as the pane's.
    fun settled(box: String, now: Long = elapsed()): String? {
        val waiting = unheard ?: return null
        if (now < quietUntil) return null
        return heard(waiting, box, now)
    }

    // The keys that make the pane's box read `after`, or null for an edit that stays in the box.
    fun typed(before: String, after: String, live: Boolean, now: Long = elapsed()): List<String>? {
        val keys = keys ?: return null
        if (!live || before != line) return null
        val shared = before.commonPrefixWith(after).length
        val tail = before.substring(shared).commonSuffixWith(after.substring(shared)).length
        val removed = before.length - shared - tail
        val inserted = after.substring(shared, after.length - tail)
        if (inserted.length > BURST) return null
        val out = mutableListOf<String>()
        walk(keys, shared + removed)?.let(out::add)
        if (removed > 0) out += keys.back.repeat(removed)
        out += pieces(inserted, keys)
        line = after
        caret = shared + inserted.length
        quietUntil = now + QUIET
        return out
    }

    // What the send button sends: the Enter alone for a pane already holding the line, and
    // otherwise the pane's line taken back a character at a time and this one typed in its place.
    fun submit(box: String, now: Long = elapsed()): List<String> {
        val keys = keys
        val held = line
        val out = mutableListOf<String>()
        when {
            keys == null || held == null -> out += box
            held.trimEnd() == box.trimEnd() -> Unit
            box.startsWith(held) && caret == held.length -> out += box.substring(held.length)
            else -> {
                walk(keys, held.length)?.let(out::add)
                if (held.isNotEmpty()) out += keys.back.repeat(held.length)
                out += box
            }
        }
        out += SUBMIT
        line = ""
        caret = 0
        quietUntil = now + QUIET
        return out.filter { it.isNotEmpty() }
    }

    private fun walk(keys: EditKeys, to: Int): String? {
        val by = to - caret
        caret = to
        return when {
            by < 0 -> keys.left.repeat(-by)
            by > 0 -> keys.right.repeat(by)
            else -> null
        }
    }

    // A newline goes as its own write: it was measured alone (#564), and inside a burst Codex
    // took it for the end of the message.
    private fun pieces(inserted: String, keys: EditKeys): List<String> =
        inserted.split('\n').flatMapIndexed { at, piece ->
            listOfNotNull(keys.newline.takeIf { at > 0 }, piece.takeIf { it.isNotEmpty() })
        }

    companion object {
        // The longest single write measured to land as typing on all three harnesses; 1500 came
        // back as a `[Pasted text]` placeholder on Claude and Codex (#564).
        const val BURST = 800

        // Past the slowest keystroke-to-reading measured through a real node (#566), so the
        // box's own keys have come back before a line that disagrees is taken for the desk's.
        const val QUIET = 1_200L

        private const val SUBMIT = "\r"
    }
}

private val origin = TimeSource.Monotonic.markNow()

private fun elapsed(): Long = origin.elapsedNow().inWholeMilliseconds
