package dev.kampr.mosaic

import androidx.compose.runtime.Immutable
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

const val MAX_CELLS = 4

// A cell narrower than this stops being a terminal and starts being a column of hyphens. Two of
// them plus the gap is the whole test for whether the window can hold a second column.
val MIN_CELL_WIDTH = 380.dp

@Immutable
data class MosaicShape(val perRow: List<Int>) {
    val cols: Int get() = perRow.maxOrNull() ?: 1
    val rows: Int get() = perRow.size
    val cells: Int get() = perRow.sum()
}

// 2x2 when the window can hold two readable columns, a single stack when it cannot. Three panes
// on two columns give the last one the full width rather than leaving a hole where a pane isn't.
fun mosaicShape(count: Int, width: Dp): MosaicShape {
    val n = count.coerceIn(1, MAX_CELLS)
    if (n == 1 || width < MIN_CELL_WIDTH * 2) return MosaicShape(List(n) { 1 })
    val rows = mutableListOf<Int>()
    var left = n
    while (left > 0) {
        rows += minOf(2, left)
        left -= 2
    }
    return MosaicShape(rows)
}

private const val SEPARATOR = ' '

// The arrangement is the ordered set of panes, and the shape follows from it and the window —
// a saved layout that also pinned a geometry would be wrong on the next screen it opened on.
fun encodeArrangement(panes: List<String>): String = panes.joinToString(SEPARATOR.toString())

fun decodeArrangement(saved: String?): List<String> =
    saved?.split(SEPARATOR)?.filter { it.isNotBlank() }?.take(MAX_CELLS).orEmpty()
