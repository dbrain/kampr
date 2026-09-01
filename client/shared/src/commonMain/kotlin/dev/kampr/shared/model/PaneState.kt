package dev.kampr.shared.model

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Facets
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn

// What the pane's own composer holds, and the keystroke measured to empty it. `clear` is null for
// a harness nobody has measured one for, which is a takeover that is not offered rather than one
// that guesses a key — and the wrong key is worse than none: `ctrl+u` takes a single visual row of
// Claude's wrapped buffer, and `ctrl+c` arms an exit on agy.
data class DeskLine(val text: String, val clear: String?)

class ScrollbackStore {
    private val rows = LinkedHashMap<Int, RowDiff>()

    var fromTop: Int = 0
        private set
    var totalRows: Int = 0
        private set
    var complete: Boolean = false
        private set
    var capped: Boolean = false
        private set

    private var highestIndex = -1

    // History arrives as one document then tails; a later message carries only new rows, so it
    // must never shrink what is already held. The node re-bases every delta's from_top onto the
    // client's known end (`send_history`), so an advanced from_top is the ordinary tail, not a
    // discard — only a start outside the held range is a gap it could not stitch. Same predicate
    // as History::absorb in kampr-mesh, which is the other end of this wire.
    fun apply(msg: ServerMsg.Scrollback) {
        val restart = msg.fromTop < fromTop || msg.fromTop > fromTop + totalRows
        if (restart) {
            fromTop = msg.fromTop
            rows.clear()
            highestIndex = -1
            totalRows = 0
        }
        complete = msg.complete
        capped = msg.capped || capped
        for (row in msg.rows) {
            rows[row.row] = row
            if (row.row > highestIndex) highestIndex = row.row
        }
        val held = if (highestIndex >= fromTop) highestIndex - fromTop + 1 else 0
        totalRows = maxOf(msg.totalRows, held, totalRows)
    }

    // Absolute ring indices run [fromTop, fromTop + totalRows); from_top advances when the
    // node discards rows across a gap, so totalRows is the depth, not the highest index.
    val historyRows: Int get() = totalRows

    fun row(index: Int): RowDiff? = rows[index]

    fun clear() {
        rows.clear()
        fromTop = 0
        totalRows = 0
        highestIndex = -1
        complete = false
        capped = false
    }
}

// Counted in herd sweeps rather than in seconds, because the sweep is what produces the evidence
// and a wall-clock threshold would only be that number in disguise. The sweep runs every 3s for as
// long as any pane is being streamed, so three separate sweeps each reporting this pane moving is
// at least six seconds of the node watching it work while handing this client not one frame —
// against a measured change-to-client latency of ~190ms, which is thirty round trips of slack.
//
// The number that matters is the floor, not the ceiling. One sweep is a race: a herd patch can
// legitimately overtake the grid frame for the same change by a sweep's width. Two is a race that
// happened twice. Three is not a race, and it still costs an idle pane nothing at all, because an
// idle pane never moves either reading this counts.
private const val QUIET_AFTER_MOVES = 3

// What the node's socket plane says happened to this pane, read off the herd it is already
// sending. Both readings are output-correlated and neither can be produced by a pane sitting
// idle: rows only enter the scrollback ring when the pane writes lines that scroll off it, and an
// agent's status only moves when the harness does something.
//
// Both, and not either alone, because the pane this was written for had neither on its own. A
// full-screen agent on the alt screen keeps no scrollback at all (its ring is flat at zero), and a
// plain shell has no agent status to move — so a detector built on one of them is blind to exactly
// half the panes in the product, and the half it was blind to was `codex`.
fun paneMoved(before: PaneInfo, after: PaneInfo): Boolean =
    after.scrollbackRows > before.scrollbackRows || after.agentStatus != before.agentStatus

// One conversation a pane's agent launched. A page of one is always `fresh` — it has nothing in
// common with the pane's transcript to merge into — but a page of it can still be asked for with a
// `before`, so the turns merge among themselves by the rule the pane's own turns use.
class SubConversation(val handle: String) {
    val turns = mutableStateListOf<Turn>()

    var cursor by mutableStateOf<String?>(null)
        private set
    var more by mutableStateOf(false)
        private set

    // True from the moment a page has landed, which is what tells an empty conversation apart
    // from one that has not answered yet.
    var loaded by mutableStateOf(false)
        private set

    fun apply(msg: ServerMsg.Convo) {
        mergeTurns(turns, msg.turns)
        cursor = msg.cursor
        more = msg.more
        loaded = true
    }

    // What the conversation has grown by since it was opened, anchored against what is drawn and
    // otherwise filed **below** it.
    //
    // Not the page's fallback. A page runs *backwards* and files an unplaceable turn above what
    // the reader holds; a transcript that is still being written runs forwards, and the same
    // fallback put the newest step at the top, above turns the agent took before it. That is the
    // pane's own `convo` / `convo.turn` distinction, and a launched conversation has both halves
    // for the same reason — including the half that reaches back past the window the reader was
    // given, which is what filed old turns under new ones (#411).
    fun apply(turns: List<Turn>) {
        mergeTurns(this.turns, turns, Unanchored.Below)
        loaded = true
    }
}

// A page merges by id, and an id the client does not hold goes where the page puts it — after the
// last turn the two have in common, before the next one. Transcript order is the node's, not the
// client's: a resumed session stamps later records with earlier times, and sorting on `at` would
// shuffle a real conversation.
//
// Unconditional prepending was the rule, written for `convo.load`, which pages *backwards*. A
// journal that is closed and re-opened on the same transcript pages *forwards*: the turns the
// client is missing are the ones written while the pump was down, and every one of them landed at
// index 0 — the newest turn in the conversation, above turns from hours earlier, on a view pinned
// to the bottom. That is a message that was never dropped and never seen, and never revisited
// either, because the node had recorded it as delivered.
//
// Prepending is still what happens when the page and the conversation share nothing, which is the
// older-page case the rule was written for and the only case where position is a guess.
// Where turns the message carries and the client has never seen go when there is nothing in it to
// anchor them against.
//
// Anchoring is the ordinary case and it is the same on both routes: an unrecognised turn belongs
// immediately before the next turn the message names that *is* recognised, because a message lists
// its turns in the transcript's own order. Only a message with no landmark in it at all has to be
// placed as a whole, and the two routes want opposite ends — `convo.load` reaches backwards for
// older turns, and the tail of a watched pane is newer.
enum class Unanchored { Above, Below }

fun mergeTurns(into: MutableList<Turn>, page: List<Turn>, unanchored: Unanchored = Unanchored.Above) {
    var after = -1
    val waiting = mutableListOf<Turn>()
    for (turn in page) {
        val at = into.indexOfFirst { it.id == turn.id }
        if (at < 0) {
            waiting += turn
            continue
        }
        into[at] = turn
        after = at
        if (waiting.isNotEmpty()) {
            into.addAll(at, waiting)
            after = at + waiting.size
            waiting.clear()
        }
    }
    if (waiting.isEmpty()) return
    val at = when {
        after >= 0 -> after + 1
        unanchored == Unanchored.Above -> 0
        else -> into.size
    }
    into.addAll(at, waiting)
}


class PaneState(val id: String, val styles: StyleTable) {
    val cells = CellBuffer(80, 24)
    val links = mutableListOf<String>()
    val scrollback = ScrollbackStore()
    val turns = mutableStateListOf<Turn>()
    private val subs = mutableStateMapOf<String, SubConversation>()

    var cursor by mutableStateOf(Cursor())
        private set
    var stale by mutableStateOf(false)

    // **Whether the transcript on screen has been confirmed against the node on this connection.**
    // The grid and the conversation are served by different machinery and only one of them is
    // warm: a pane's stream is held across a re-watch by the registry (#252), so the terminal
    // reattaches to a live emulator, while the conversation is opened inside the pump that the
    // watch created and is cold every time — resolve, fold, page. Nothing pruned the turns already
    // drawn in that gap, and `stale` could not cover it because `stale` is the grid's reading:
    // gated on `painted` and cleared by the next frame, which arrives long before the page does.
    // So a conversation from a session that had been `/clear`ed sat there looking live (#393).
    var convoConfirmed by mutableStateOf(false)
        internal set
    var painted by mutableStateOf(false)
        private set
    var pending by mutableStateOf<ServerMsg.Pending?>(null)
        internal set
    // A reply half written and not yet sent. It belongs to the pane rather than to the composer,
    // which leaves the composition every time the operator looks at the terminal.
    //
    // **Kept across a dropped socket, unlike `pending`.** A question is dropped because answering
    // a stale one puts a key into a pane with nothing matching to take it; a draft is the
    // operator's own sentence, and losing it to a blink of the network is the complaint this
    // exists to answer.
    var draft by mutableStateOf("")

    var convoCursor by mutableStateOf<String?>(null)
        private set
    var convoMore by mutableStateOf(false)
        private set

    var revision by mutableIntStateOf(0)
        private set

    // How many times the node's *other* half has reported this pane moving with no frame arriving
    // in between. See [`paneMoved`] for what counts as moving and why an idle pane cannot.
    var unshownMoves by mutableIntStateOf(0)
        private set

    // **A pane whose frames have stopped while its connection is healthy.** This is the state the
    // browser report was made of and the one nothing in the client could see: the socket was up,
    // the herd list was fresh, this pane's own conversation was answering on that same socket, and
    // its grid sat on a screen minutes old. Every existing signal is downstream of the socket
    // dying — `stale` means "the socket went away since we last painted" — so all of them said the
    // pane was fine.
    //
    // It is not a timeout, and that is the whole of why it is safe. A pane nobody is typing in is
    // legitimately silent for hours, so elapsed silence is not evidence of anything. What this
    // counts is a *contradiction*: the node's socket plane keeps saying this pane is doing things
    // while the node's stream plane delivers nothing, and the two planes fail independently
    // (#233). An idle pane contributes nothing to either side of it.
    val quiet: Boolean
        get() = painted && !stale && unshownMoves >= QUIET_AFTER_MOVES

    fun noteMoved() {
        if (painted) unshownMoves++
    }

    // The node saying outright what the count above can only infer: this pane's frames have
    // stopped. It is latched into the same state rather than shown as its own notice because the
    // notice does not survive — `dropRepairedFault` clears a `stream_unavailable` as soon as the
    // pane's herd entry carries no `detail`, which a per-pane stream death never sets, so the
    // banner would be taken down by the next sweep three seconds later. One vocabulary, and it
    // clears the way everything else here does: when a frame arrives.
    fun noteStreamStopped() {
        if (painted) unshownMoves = QUIET_AFTER_MOVES
    }

    // Keystrokes that never left the device. Counted rather than flagged, because "nothing you
    // typed for the last thirty seconds arrived" is a different fact from "one key was lost".
    var undelivered by mutableIntStateOf(0)
        private set

    fun noteUndelivered() {
        undelivered++
    }

    fun noteDelivered() {
        if (undelivered != 0) undelivered = 0
    }

    fun applyReset(msg: ServerMsg.GridReset) {
        cells.resize(msg.cols, msg.rows)
        cells.clear()
        for (row in msg.rowsData) cells.apply(row)
        cursor = msg.cursor
        // A reset carries the pane's whole link table from index 0, because a full repaint clears
        // herdr's; only a patch carries the suffix.
        links.clear()
        links += msg.links
        painted = true
        stale = false
        unshownMoves = 0
        revision++
    }

    fun applyPatch(msg: ServerMsg.GridPatch) {
        for (row in msg.rows) cells.apply(row)
        msg.cursor?.let { cursor = it }
        links += msg.links
        stale = false
        unshownMoves = 0
        revision++
    }

    fun applyScrollback(msg: ServerMsg.Scrollback) {
        scrollback.apply(msg)
        revision++
    }

    fun applyConvo(msg: ServerMsg.Convo) {
        convoConfirmed = true
        mergeTurns(turns, msg.turns)
        convoCursor = msg.cursor
        convoMore = msg.more
        revision++
    }

    // A conversation the pane's agent launched, held apart from the pane's own turns and keyed by
    // the opaque handle the `sub` block carried. Apart is the whole of it: a launched conversation
    // shares no turn id with the pane's, so merging one in would put a subagent's words in the
    // transcript as the pane's own reply — the one thing the wire went out of its way to prevent
    // by refusing to inline them.
    fun applySubConvo(msg: ServerMsg.Convo) {
        val handle = msg.sub ?: return
        sub(handle).apply(msg)
        revision++
    }

    fun sub(handle: String): SubConversation = subs.getOrPut(handle) { SubConversation(handle) }

    fun subOrNull(handle: String): SubConversation? = subs[handle]

    // What the harness has recorded about the session, replaced wholesale by every frame that
    // carries it. See [`ServerMsg.ConvoFacets`]: the node republishes when the queue moves, so the
    // newest frame *is* the queue and merging one in would leave a delivered prompt standing.
    //
    // Kept across a dropped socket, unlike `pending`. The queue is a description of the pane and
    // not a question waiting on this client's answer, so a stale one is dated rather than wrong —
    // the same rule the turns are held under — and re-opening the conversation republishes it.
    var facets by mutableStateOf(Facets())
        private set

    fun applyFacets(msg: ServerMsg.ConvoFacets) {
        facets = msg.facets
        revision++
    }

    // What is sitting in the pane's own composer, and the key measured to take it out. Null is an
    // empty box: the node publishes `text: null` when the desk empties it, and holding the last
    // line instead would leave the strip claiming a sentence that is no longer there.
    //
    // Kept across a dropped socket for the same reason `facets` is — it describes the pane rather
    // than asking this client anything, so a stale one is dated and not wrong.
    var desk by mutableStateOf<DeskLine?>(null)
        private set

    fun applyComposer(msg: ServerMsg.ConvoComposer) {
        desk = msg.text?.let { DeskLine(it, msg.clear) }
        revision++
    }

    fun applyConvoTurn(msg: ServerMsg.ConvoTurn) {
        // **A launched conversation's turns are never the pane's own.** Appending them here would
        // put a subagent's words in the parent's voice, which is the one thing the whole `sub`
        // shape exists to prevent — and it is the shape a growing transcript arrives in, so this
        // is the path it would happen on.
        msg.sub?.let { handle ->
            sub(handle).apply(msg.turns)
            revision++
            return
        }
        // **Not an append.** A revision is what a re-watch is served, and the node's page runs
        // back past its own bound to the question that opens the reply it landed in — so a
        // revision routinely names turns older than the window this client is holding. Filed at
        // the end, those became the newest thing on a view that pins to its own end, and the
        // reader was left looking at a message from an hour before (#411).
        mergeTurns(turns, msg.turns, Unanchored.Below)
        revision++
    }

    // The last thing the node refused about this pane, in its own words. Held here so it can be
    // said on the pane it is about rather than only over whatever screen happened to be open — a
    // refusal about a pane the operator is not looking at is news for when they arrive at it.
    var refusal: String? by mutableStateOf(null)
        internal set

    fun clearRefusal() {
        refusal = null
    }

    fun markStale() {
        refusal = null
        if (painted) stale = true
        convoConfirmed = false
        // Nothing carried across a dropped socket is trustworthy, and a question least of all: the
        // node publishes `pending` on a blocked-state edge and its first attempt at a newly blocked
        // pane carries nothing, so a reconnect would triage the previous connection's question and
        // answer it into a pane with nothing matching to answer.
        pending = null
    }
}
