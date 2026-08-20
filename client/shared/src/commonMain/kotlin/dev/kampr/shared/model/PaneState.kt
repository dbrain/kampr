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
    // must never shrink what is already held. from_top only ever advances, on a discard.
    fun apply(msg: ServerMsg.Scrollback) {
        val discarded = msg.fromTop > fromTop
        if (discarded) {
            fromTop = msg.fromTop
            val dropped = rows.keys.filter { it < fromTop }
            for (key in dropped) rows.remove(key)
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

    fun applyReset(msg: ServerMsg.GridReset) {
        cells.resize(msg.cols, msg.rows)
        cells.clear()
        for (row in msg.rowsData) cells.apply(row)
        cursor = msg.cursor
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

    fun applyConvo(msg: ServerMsg.Convo) {
        val existing = turns.associateBy { it.id }
        val merged = (msg.turns + turns).distinctBy { it.id }.sortedBy { it.at ?: "" }
        turns.clear()
        turns.addAll(merged.map { existing[it.id] ?: it })
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
