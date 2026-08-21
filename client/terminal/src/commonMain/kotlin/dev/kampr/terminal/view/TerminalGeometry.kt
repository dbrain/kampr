package dev.kampr.terminal.view

import dev.kampr.shared.model.surfaceGeometry
import kotlin.math.max
import kotlin.math.min

const val BASE_CELL_SP = 13f

// Paint and content are two different rectangles. The terminal paints the whole viewport so rows
// run under the header and the key row and nothing is ever blank; the scrollable content is inset
// by that chrome so the pinned last row settles clear of it. Fill is computed against the paint
// rectangle — insetting it first is what reintroduces the letterbox the insets exist to avoid.
data class PaintRect(
    val width: Float,
    val height: Float,
    val insetTop: Float,
    val insetBottom: Float,
) {
    val contentHeight: Float get() = (height - insetTop - insetBottom).coerceAtLeast(1f)
    val contentBottom: Float get() = height - insetBottom
}

data class TerminalGeometry(
    val originX: Float,
    val originY: Float,
    val panX: Float,
    val scrollY: Float,
    val maxScroll: Float,
    val minPanX: Float,
    val surfaceHeight: Float,
    val gridWidth: Float,
) {
    val pinned: Boolean get() = scrollY <= 0.5f
}

fun terminalGeometry(
    paint: PaintRect,
    cols: Int,
    totalRows: Int,
    cellWidth: Float,
    cellHeight: Float,
    panX: Float,
    scrollY: Float,
    // Room above row 0 for the mark that says where the record stops. Without it the top row sits
    // flush under the header at full scroll and there is nowhere for the mark to be.
    topPad: Float = 0f,
): TerminalGeometry {
    val gridWidth = cols * cellWidth
    val surfaceHeight = totalRows * cellHeight
    val minPanX = min(0f, paint.width - gridWidth)
    val clampedPan = panX.coerceIn(minPanX, 0f)
    val maxScroll = max(0f, surfaceHeight - paint.contentHeight + topPad)
    val clampedScroll = scrollY.coerceIn(0f, maxScroll)
    return TerminalGeometry(
        originX = clampedPan,
        originY = paint.contentBottom - surfaceHeight + clampedScroll,
        panX = clampedPan,
        scrollY = clampedScroll,
        maxScroll = maxScroll,
        minPanX = minPanX,
        surfaceHeight = surfaceHeight,
        gridWidth = gridWidth,
    )
}

data class ZoomPresets(val fitWidth: Float, val readable: Float, val closeUp: Float) {
    val minimum: Float get() = min(fitWidth, readable) * 0.5f
    val maximum: Float get() = closeUp * 2.5f
}

fun zoomPresets(paintWidth: Float, cols: Int, baseCellWidth: Float): ZoomPresets {
    val fit = if (cols > 0 && baseCellWidth > 0f) paintWidth / (cols * baseCellWidth) else 1f
    return ZoomPresets(
        fitWidth = fit.coerceIn(0.05f, 12f),
        readable = READABLE_SP / BASE_CELL_SP,
        closeUp = CLOSE_UP_SP / BASE_CELL_SP,
    )
}

// max(fit-width, fit-height), never min: fitting inside both axes is exactly what leaves blank
// space below the last row. The rows available to fill the height are history plus the live
// viewport, because the space above a short grid carries history rather than nothing — on an
// alt-screen pane with no ring this collapses back to max(fit-width, fit-height).
fun defaultZoom(
    paint: PaintRect,
    cols: Int,
    liveRows: Int,
    historyRows: Int,
    baseCellWidth: Float,
    baseCellHeight: Float,
): Float = surfaceGeometry(
    viewportWidth = paint.width,
    viewportHeight = paint.height,
    cols = cols,
    liveRows = liveRows + historyRows.coerceAtLeast(0),
    historyRows = 0,
    cellWidth = baseCellWidth,
    cellHeight = baseCellHeight,
).zoom

// A pane taller than the viewport opens on the caret rather than on whatever happens to sit at the
// bottom of the grid: an alt-screen app draws its prompt wherever it likes, and pinning blindly can
// open on its blank tail.
fun initialScroll(
    paint: PaintRect,
    totalRows: Int,
    cursorIndex: Int,
    cellHeight: Float,
): Float {
    val surfaceHeight = totalRows * cellHeight
    val maxScroll = max(0f, surfaceHeight - paint.contentHeight)
    if (maxScroll <= 0f) return 0f
    val pinnedTop = paint.contentBottom - surfaceHeight + cursorIndex * cellHeight
    if (pinnedTop >= paint.insetTop) return 0f
    val target = paint.contentBottom - 4f * cellHeight
    return (target - pinnedTop).coerceIn(0f, maxScroll)
}

private const val READABLE_SP = 15f
private const val CLOSE_UP_SP = 22f

// Follow-cursor only nudges horizontally: the live viewport is already pinned to the bottom
// unless the operator has scrolled away, and scrolling away is a deliberate act to preserve.
fun followCursorPan(
    panX: Float,
    minPanX: Float,
    cursorCol: Int,
    cellWidth: Float,
    viewWidth: Float,
): Float {
    if (minPanX >= 0f) return 0f
    val margin = cellWidth * 4f
    val left = cursorCol * cellWidth
    val right = left + cellWidth
    val target = when {
        left + panX < margin -> margin - left
        right + panX > viewWidth - margin -> viewWidth - margin - right
        else -> panX
    }
    return target.coerceIn(minPanX, 0f)
}
