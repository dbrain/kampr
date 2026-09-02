package dev.kampr.terminal.review

import dev.kampr.terminal.render.LogicalText
import dev.kampr.terminal.render.SurfaceRows

// What review is allowed to know about the pane: rows by index, the ring's own coordinates, and
// where the pane's own caret is. Rebuilt per call — it holds nothing, so it cannot go stale.
class ReviewSurface(
    private val rows: SurfaceRows,
    private val logical: LogicalText,
    val cursorIndex: Int,
) {
    val total: Int get() = rows.total
    val historyRows: Int get() = rows.historyRows
    val fromTop: Int get() = rows.fromTop
    val capped: Boolean get() = rows.capped
    val complete: Boolean get() = rows.complete

    fun rowAt(index: Int): String = logical.rowAt(index)
}

enum class ReviewMove { PreviousLine, NextLine, PreviousWord, NextWord, Reread, Now }

// A history row keeps its absolute ring index while more history arrives above the live grid; a
// live row does not keep anything, because the pane repaints it. Anchoring to the right one of
// these is the whole difference between a reader who stays where they parked and one who does not.
sealed interface ReviewAnchor {
    data class History(val absolute: Int) : ReviewAnchor
    data class Live(val row: Int) : ReviewAnchor
}

// `complete` is `from_top == 0` and `capped` is "something above was unreachable", so the two are
// independent and mean different losses. Naming them apart is the point of this type.
enum class HistoryEdge { None, Whole, Clipped, Discarded, Superseded }

fun historyEdge(surface: ReviewSurface): HistoryEdge = when {
    // Asked before the plain empty case, which used to swallow it. A harness that takes the
    // alternate screen takes herdr's ring with it (#438) and the node stops vouching for the shell
    // session that ran before it — nothing held, `complete` gone, `capped` set, which is the node
    // saying "there was more and it cannot be reached". Reading the row count first turned that
    // into "this pane keeps no history", which is the node's one careful admission rendered as its
    // opposite. `capped` is what separates the two: a pane that never had a ring is not capped.
    surface.historyRows == 0 && surface.capped -> HistoryEdge.Superseded
    surface.historyRows == 0 -> HistoryEdge.None
    !surface.complete -> HistoryEdge.Discarded
    surface.capped -> HistoryEdge.Clipped
    else -> HistoryEdge.Whole
}

fun historyEdgeSpoken(surface: ReviewSurface): String = when (historyEdge(surface)) {
    HistoryEdge.None -> "Top of the grid. This pane keeps no history."
    HistoryEdge.Superseded ->
        "Top of the grid. The program on screen keeps its own scrollback, so Kampr holds none for " +
            "this pane while it is running."
    HistoryEdge.Whole -> "Top of the history. The record starts here."
    HistoryEdge.Clipped ->
        "Top of the history. Older output was never captured — herdr hands back at most 1000 " +
            "lines and cannot be asked for more."
    HistoryEdge.Discarded ->
        "Top of the history. ${surface.fromTop} rows above this were discarded when output " +
            "outran the poll, and cannot be fetched back."
}

// Quiet when the record is whole. A badge that is always there is a badge nobody reads, and the
// only thing worth saying about intact history is nothing.
fun historyWarning(surface: ReviewSurface): String? = when (historyEdge(surface)) {
    HistoryEdge.None, HistoryEdge.Whole -> null
    HistoryEdge.Superseded ->
        "the program on screen keeps its own scrollback — Kampr holds none for this pane while it " +
            "is running, and what ran before it is no longer this pane's history"
    HistoryEdge.Clipped ->
        "history is clipped — output older than this scrolled away before Kampr was watching, " +
            "and herdr caps a read at 1000 lines"
    HistoryEdge.Discarded ->
        "history is broken — ${surface.fromTop} rows were discarded here because output outran " +
            "the poll, and nothing can fetch them back"
}

// Short enough to sit at the top of the surface without becoming a paragraph.
fun historyEdgeLabel(surface: ReviewSurface): String? = when (historyEdge(surface)) {
    HistoryEdge.None -> null
    HistoryEdge.Superseded -> "the program keeps its own history"
    HistoryEdge.Whole -> "history starts here"
    HistoryEdge.Clipped -> "older output is unreachable"
    HistoryEdge.Discarded -> "${surface.fromTop} rows lost here"
}

private val SPACES = Regex("\\s+")

fun reviewWords(text: String): List<String> =
    text.split(SPACES).filter { it.isNotBlank() }
