package dev.kampr.shared.model

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.RowDiff
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
        revision++
    }

    fun applyPatch(msg: ServerMsg.GridPatch) {
        for (row in msg.rows) cells.apply(row)
        msg.cursor?.let { cursor = it }
        links += msg.links
        stale = false
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
    }
}
