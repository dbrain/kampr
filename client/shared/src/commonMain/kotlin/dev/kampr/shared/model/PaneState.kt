package dev.kampr.shared.model

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn

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

class PaneState(val id: String, val styles: StyleTable) {
    val cells = CellBuffer(80, 24)
    val links = mutableListOf<String>()
    val scrollback = ScrollbackStore()
    val turns = mutableStateListOf<Turn>()

    var cursor by mutableStateOf(Cursor())
        private set
    var stale by mutableStateOf(false)
        internal set
    var painted by mutableStateOf(false)
        private set
    var pending by mutableStateOf<ServerMsg.Pending?>(null)
        internal set
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

    // A page is a slice of the transcript running backwards from where the client already is, so
    // it prepends. Transcript order is the node's, not the client's: a resumed session stamps
    // later records with earlier times, and sorting on `at` would shuffle a real conversation.
    fun applyConvo(msg: ServerMsg.Convo) {
        val older = mutableListOf<Turn>()
        for (turn in msg.turns) {
            val at = turns.indexOfFirst { it.id == turn.id }
            if (at >= 0) turns[at] = turn else older += turn
        }
        turns.addAll(0, older)
        convoCursor = msg.cursor
        convoMore = msg.more
        revision++
    }

    fun applyConvoTurn(msg: ServerMsg.ConvoTurn) {
        for (turn in msg.turns) {
            val at = turns.indexOfFirst { it.id == turn.id }
            if (at >= 0) turns[at] = turn else turns.add(turn)
        }
        revision++
    }

    fun markStale() {
        if (painted) stale = true
        // Nothing carried across a dropped socket is trustworthy, and a question least of all: the
        // node publishes `pending` on a blocked-state edge and its first attempt at a newly blocked
        // pane carries nothing, so a reconnect would triage the previous connection's question and
        // answer it into a pane with nothing matching to answer.
        pending = null
    }
}
