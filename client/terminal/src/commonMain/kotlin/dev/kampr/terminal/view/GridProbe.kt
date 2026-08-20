package dev.kampr.terminal.view

import androidx.compose.ui.geometry.Offset
import dev.kampr.terminal.render.GridPoint
import kotlin.math.floor

// Pointer positions have to become cells against the geometry of the frame the finger is on, and
// that geometry moves with every pan. A gesture detector keyed on it would restart mid-drag, so it
// reads this holder instead.
class GridProbe {
    var originX = 0f
    var originY = 0f
    var cellWidth = 1f
    var cellHeight = 1f
    var cols = 1
    var totalRows = 1

    fun cellAt(position: Offset): GridPoint {
        val col = floor((position.x - originX) / cellWidth).toInt().coerceIn(0, cols - 1)
        val row = floor((position.y - originY) / cellHeight).toInt().coerceIn(0, totalRows - 1)
        return GridPoint(row, col)
    }
}
