package dev.kampr.shared.model

import kotlin.math.max

data class SurfaceGeometry(
    val zoom: Float,
    val originY: Float,
    val surfaceHeight: Float,
    val viewportHeight: Float,
) {
    val letterboxed: Boolean get() = surfaceHeight < viewportHeight - 0.5f
}

// Scrollback and the live grid are one continuous surface: history runs off the top, the live
// viewport is pinned to the bottom. Zoom fills at least one axis — max, never min, because
// fitting inside both axes is exactly what leaves blank space below the last row.
fun surfaceGeometry(
    viewportWidth: Float,
    viewportHeight: Float,
    cols: Int,
    liveRows: Int,
    historyRows: Int,
    cellWidth: Float,
    cellHeight: Float,
): SurfaceGeometry {
    if (cols <= 0 || liveRows <= 0 || cellWidth <= 0f || cellHeight <= 0f) {
        return SurfaceGeometry(1f, 0f, viewportHeight, viewportHeight)
    }
    val zoom = max(viewportWidth / (cols * cellWidth), viewportHeight / (liveRows * cellHeight))
    val rowHeight = cellHeight * zoom
    val surfaceHeight = (historyRows.coerceAtLeast(0) + liveRows) * rowHeight
    val originY = (viewportHeight - surfaceHeight).coerceAtMost(0f)
    return SurfaceGeometry(zoom, originY, surfaceHeight, viewportHeight)
}
