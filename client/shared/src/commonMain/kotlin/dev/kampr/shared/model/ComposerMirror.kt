package dev.kampr.shared.model

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
    // A member, never a top-level property: on Kotlin/Wasm one top-level initialiser in this file
    // moves the class's Compose `$stable` into the file's lazy initialiser, which the `$stable`
    // getter does not run, so the reply box's first composition and its next read it differently
    // and misread their slots (BrowserComposerFirstFrameTest).
    private val origin = TimeSource.Monotonic.markNow()
    private var line: String? = null
    private var caret = 0
    private var keys: EditKeys? = null

    // A reading with no box is a pane that cannot be read this moment, not one that has forgotten
    // what was typed into it: Claude repaints with its caret off the box (#552).
    private var measured: EditKeys? = null
    private var quietUntil = 0L
    private var unheard: DeskLine? = null

    // The reading `line` was last taken from. A reply box built again (a look at the terminal or
    // at another pane) hears the pane's last reading once more, and one already taken is older
    // than every key typed since: taken again, it read as the desk emptying the box.
    private var taken: DeskLine? = null

    // What each `[Image #n]` this box placed was pasted as, so a send that has to type one again
    // pastes the file rather than typing its name. Learned when the token comes back as the line.
    private val placing = ArrayDeque<String>()
    private val pictures = mutableMapOf<String, String>()

    val at: Int get() = caret

    val images: Boolean get() = (keys ?: measured)?.images == true

    // What the box should now hold, or null to leave it alone.
    fun heard(desk: DeskLine?, box: String, now: Long = elapsed()): String? {
        keys = desk?.keys
        desk?.keys?.let { measured = it }
        if (desk == null || desk.keys == null || desk == taken) return null
        if (now < quietUntil && desk.text != line) {
            unheard = desk
            return null
        }
        unheard = null
        taken = desk
        val inStep = box == (line ?: "")
        val held = line
        if (inStep && held != null && scrolled(held, desk.text)) {
            caret = (held.length - (desk.text.length - (desk.caret ?: desk.text.length))).coerceIn(0, held.length)
            return null
        }
        learn(line, desk.text)
        line = desk.text
        caret = (desk.caret ?: desk.text.length).coerceIn(0, desk.text.length)
        return desk.text.takeIf { inStep && it != box }
    }

    // Whether a reading that differs from the box is only the box's own keys still on their way
    // back. A box in step holds the pane's line as those keys leave it, so anything else the pane
    // reads is either that lag or the desk's own typing, which `settled` is about to take up.
    fun echoing(desk: DeskLine, box: String): Boolean = desk.keys != null && box == line

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
        if (!keys.paced && inserted.length > BURST) return null
        val out = mutableListOf<String>()
        walk(keys, before, shared + removed)?.let(out::add)
        if (removed > 0) out += keys.back.repeat(steps(keys, before, shared, shared + removed))
        out += pieces(inserted, keys)
        line = after
        caret = shared + inserted.length
        quietUntil = now + QUIET + arriving(inserted)
        return out
    }

    // What the send button sends: the Enter alone for a pane already holding the line, and
    // otherwise the pane's line taken back a character at a time and this one typed in its place.
    fun submit(box: String, now: Long = elapsed()): List<String> {
        val keys = keys ?: measured
        val held = line
        val out = mutableListOf<String>()
        when {
            keys == null || held == null -> out += cut(box, keys)

            held.trimEnd() == box.trimEnd() -> Unit

            box.startsWith(held) && caret == held.length -> out += retyped(box.substring(held.length), keys)

            // Only where the two differ: a whole line taken back and typed again lost the pictures
            // in it and had its Enter kept as a newline (#619).
            else -> {
                val shared = held.commonPrefixWith(box).length
                val tail = held.substring(shared).commonSuffixWith(box.substring(shared)).length
                val removed = held.length - shared - tail
                walk(keys, held, shared + removed)?.let(out::add)
                if (removed > 0) out += keys.back.repeat(steps(keys, held, shared, shared + removed))
                out += retyped(box.substring(shared, box.length - tail), keys)
            }
        }
        out += SUBMIT
        line = ""
        caret = 0
        quietUntil = now + QUIET + arriving(box)
        return out.filter { it.isNotEmpty() }
    }

    // The keys that put the pane's caret where a file is to land, into a box in step that may
    // type, and a space when that caret is against a word. The node types the file at the pane's
    // caret, a harness that draws a picture draws its token there (#620), and what it typed comes
    // back as the pane's line once these keys have settled.
    fun place(box: String, at: Int, live: Boolean, now: Long = elapsed()): List<String>? {
        val keys = keys ?: return null
        if (!live || box != line) return null
        val to = IMAGE_TOKEN.findAll(box).firstOrNull { at > it.range.first && at <= it.range.last }?.let { it.range.last + 1 } ?: at
        quietUntil = now + QUIET
        val apart = " ".takeIf { to > 0 && !box[to - 1].isWhitespace() }
        return listOfNotNull(walk(keys, box, to), apart)
    }

    // What the node typed for a file this box placed. Only a picture is pasted bracketed, and only
    // a picture draws a token: the next new one the pane's line shows is that file.
    fun placed(text: String) {
        if (text.startsWith(BRACKETED_PASTE)) placing.addLast(text)
    }

    private fun learn(before: String?, after: String) {
        if (after.isBlank()) pictures.clear()
        if (placing.isEmpty()) return
        val had = before?.let { IMAGE_TOKEN.findAll(it).map { token -> token.value }.toSet() }.orEmpty()
        IMAGE_TOKEN.findAll(after).map { it.value }.filter { it !in had }.forEach { token ->
            placing.removeFirstOrNull()?.let { pictures[token] = it }
        }
    }

    private fun retyped(text: String, keys: EditKeys): List<String> {
        val out = mutableListOf<String>()
        var at = 0
        for (token in IMAGE_TOKEN.findAll(text)) {
            val pasted = pictures[token.value] ?: continue
            out += cut(text.substring(at, token.range.first), keys)
            out += pasted
            at = token.range.last + 1
        }
        out += cut(text.substring(at), keys)
        return out
    }

    private fun elapsed(): Long = origin.elapsedNow().inWholeMilliseconds

    private fun walk(keys: EditKeys, text: String, to: Int): String? {
        val by = steps(keys, text, minOf(caret, to), maxOf(caret, to))
        val left = to < caret
        caret = to
        return when {
            by == 0 -> null
            left -> keys.left.repeat(by)
            else -> keys.right.repeat(by)
        }
    }

    // A newline goes as its own write: it was measured alone (#564), and inside a burst Codex
    // took it for the end of the message.
    private fun pieces(inserted: String, keys: EditKeys): List<String> = inserted.split('\n').flatMapIndexed { at, piece ->
        listOfNotNull(keys.newline.takeIf { at > 0 }) + cut(piece, keys)
    }

    // Never between the two halves of a surrogate pair, which would be two writes of invalid text.
    // A node that does not pace them writes the pieces back to back, which is the paste they are
    // cut to avoid, so a box reaching one keeps its words whole.
    private fun cut(text: String, keys: EditKeys?): List<String> {
        if (keys?.paced != true) return listOfNotNull(text.takeIf { it.isNotEmpty() })
        val out = mutableListOf<String>()
        var at = 0
        while (at < text.length) {
            var end = minOf(at + PIECE, text.length)
            if (end < text.length && text[end - 1].isHighSurrogate()) end--
            out += text.substring(at, end)
            at = end
        }
        return out
    }

    // A box taller than the harness lets it grow scrolls, and the pane reads only its last words,
    // a newline at the cut sometimes read as a space (#666).
    private fun scrolled(held: String, read: String): Boolean {
        val seen = words(read)
        return seen.isNotEmpty() && read.length < held.length && words(held).endsWith(seen)
    }

    private fun words(text: String): String = text.split(WHITESPACE).filter { it.isNotEmpty() }.joinToString(" ")

    // How long the node takes to type `text` at its pace, which the pane's readings lag behind.
    private fun arriving(text: String): Long = text.encodeToByteArray().size * 1_000L / REPLY_BYTES_PER_SEC

    companion object {
        // The longest single write measured to land as typing on all three harnesses; 1500 came
        // back as a `[Pasted text]` placeholder on Claude and Codex (#564). Only a node that paces
        // a reply's writes (#666) is typed a longer paste.
        const val BURST = 800

        // The writes a reply is cut into: back to back, two of them drew a paste placeholder on
        // Claude and Codex, and at the node's pace none did (#666).
        const val PIECE = 400

        // kampr-core's `REPLY_BYTES_PER_SEC`, the pace the node types a reply's pieces at.
        private const val REPLY_BYTES_PER_SEC = 2_000L

        // Past the slowest keystroke-to-reading measured through a real node (#566), so the
        // box's own keys have come back before a line that disagrees is taken for the desk's.
        const val QUIET = 1_200L

        private const val SUBMIT = "\r"

        private val WHITESPACE = Regex("\\s+")
    }
}
