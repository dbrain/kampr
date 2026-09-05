package dev.kampr.terminal.view

import dev.kampr.shared.model.surfaceGeometry
import kotlin.math.max
import kotlin.math.min

const val BASE_CELL_SP = 13f

// The grid this view would show a pane at, in cells of whatever size the caller hands over.
//
// **The cell has to be a constant, and which constant is the caller's to know** (ADR 0013). The fit
// ladder changes the zoom to suit the pane's width, so while the zoom is derived a number taken at
// the current cell size is a function of the pane — ask for it, the pane moves, the zoom moves, ask
// again; there the base cell is the only safe reference. A zoom the *operator* chose is a constant
// like any other, and measuring in its cells is what makes the answer true of the screen as well as
// pure: a grid counted in base cells and drawn at 1.2x is a fifth taller than the rectangle
// drawing it.
fun viewGrid(paint: PaintRect, cellWidth: Float, cellHeight: Float): Pair<Int, Int> = Pair(
    (paint.width / cellWidth).toInt().coerceAtLeast(1),
    (paint.contentHeight / cellHeight).toInt().coerceAtLeast(1),
)

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
)

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

// How a zoom is spoken and written wherever a person reads one — the button, the sheet's header,
// the slider. Distinct from `TerminalView`'s two-decimal one, which is not a label at all: that is
// the value written into the `zoom` pref and it goes on the wire.
internal fun zoomLabel(zoom: Float): String {
    if (zoom <= 0f) return "fit"
    val tenths = (zoom * 10f + 0.5f).toInt()
    return "${tenths / 10}.${tenths % 10}×"
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
//
// `ceiling` is for the caller whose viewport is much bigger than the grid, which on a desktop is
// the ordinary case: a fresh 40x12 pane in a 1624x1000 window fills to 3.6x, legible long before
// it got there. It caps the magnification only, and it belongs to the caller because it is a fact
// about the surface and not about the grid. A phone has no room to spare: capping there
// letterboxes, which is what the max above prevents.
//
// `floor` is the same argument the other way up, and it is the operator's: *"default is often 0.4x
// and is tiny tiny … maybe we push towards 1.0x being at least default"*. Fitting a wide pane into
// a window is only worth doing while the result can be read — a 300-column pane on a desk fits at
// 0.7x, which is 13sp of text at nine — and the whole pane at a size nobody can read is not a view
// of it. The caller decides where that stops mattering, because it is the *window* that decides:
// below the width where 1.0x still leaves a usable pane on the screen, a floor would pin a phone
// to a fifth of a pane instead, and Fit width is what it is for.
fun defaultZoom(
    paint: PaintRect,
    cols: Int,
    liveRows: Int,
    historyRows: Int,
    baseCellWidth: Float,
    baseCellHeight: Float,
    ceiling: Float = Float.MAX_VALUE,
    floor: Float = 0f,
): Float = min(
    ceiling,
    surfaceGeometry(
        viewportWidth = paint.width,
        viewportHeight = paint.height,
        cols = cols,
        liveRows = liveRows + historyRows.coerceAtLeast(0),
        historyRows = 0,
        cellWidth = baseCellWidth,
        cellHeight = baseCellHeight,
    ).zoom,
).coerceAtLeast(min(floor, ceiling))

// Where the surface may rest while it is following: the band of scroll values that leave the
// caret inside the content rectangle *and* the end of the record no higher than the bottom of it,
// floor first.
//
// The floor is the least such scroll, and zero whenever the grid already fits. Pinning the bottom
// of the grid to the bottom of the rectangle is right only while it does. A herdr pane is as tall
// as the desktop made it, the caret sits wherever the shell left it — near the top of a freshly
// started one — and the rectangle is shorter than the grid the moment the keyboard is up.
// Bottom-pinning then shows the blank tail and takes the caret, the prompt, and every character
// being typed off the top with it.
//
// **A band rather than a point, because the floor is a minimum and not a place.** Resting exactly
// on it hands the caret the viewport: every frame that moves the caret moves the surface by the
// whole distance, in both directions. That is what an in-place redraw does several times a second
// — a `docker compose pull` walks the caret to the top of its block, rewrites every line and
// returns — and the operator watched the output they were reading swing off the screen and back
// seven rows at a time (#380).
// Inside the band nothing moves; outside it the surface moves the least it can, which is what
// keeping the caret on screen actually asks for.
//
// **The band is a function of one number: how many rows sit below the caret.** Its width is fixed
// at `contentHeight - cellHeight`, so it translates with the caret one pixel per pixel — and a
// caret excursion wider than the band therefore drags the viewport in *both* directions, once per
// frame. A full-screen redraw is exactly that excursion: on a grid taller than the rectangle,
// #380's fix does not cover it and the surface swings up and back several times a second. So the
// index handed in is the one the caret has *held still on*, never the live one (`TerminalView`).
//
// That the band depends on the caret's distance from the bottom rather than on its absolute index
// is what makes settling safe on a pane whose output scrolls: the caret stays on the last live row
// while its index grows with every row that leaves the grid, so the distance — and the band — never
// move, and a follower is never dragged by a reading that has gone stale.
data class CaretBand(val floor: Float, val ceiling: Float)

// The end of what there is to read, as a scroll: the last written row of the surface sitting on
// the bottom of the content rectangle. Nothing below it is anything — the rows are there because
// the desk made the pane that tall, and they are blank.
//
// It is the floor of a *hand*, which the caret's floor is not and never was (#428). Both floors
// exist because they answer different questions and disagree in both directions: a shell pane's
// content stops at the caret, so the caret floor sits a whole screenful *below* the end of the
// record and would strand a reader in the tail; a full-screen redraw writes rows underneath a
// caret that stayed put, so the caret floor sits *above* the end of it and would put the last
// rows of the pane out of reach, which is exactly the defect #428 fixed.
//
// Clamped to `maxScroll` for the pane with less in it than the rectangle can show — four lines in
// a ninety-row window — where the end of the content is above the top of the surface's travel and
// the honest answer is that there is nowhere to go at all. A grid that fits its rectangle has no
// travel to clamp and answers zero, as it did before there was a floor of any kind.
// `reserved` is history the node is holding for this pane that has not been delivered — see
// `TerminalView`'s `deepestRing`. It is travel like any other row above the grid: the surface may
// go there, and what is drawn there is blank until the rows arrive.
fun contentFloor(
    paint: PaintRect,
    totalRows: Int,
    contentIndex: Int,
    cellHeight: Float,
    reserved: Float = 0f,
): Float {
    val maxScroll = max(0f, totalRows * cellHeight - paint.contentHeight + reserved)
    return ((totalRows - 1 - contentIndex) * cellHeight).coerceIn(0f, maxScroll)
}

fun caretBand(
    paint: PaintRect,
    totalRows: Int,
    cursorIndex: Int,
    contentIndex: Int,
    cellHeight: Float,
    reserved: Float = 0f,
): CaretBand {
    val surfaceHeight = totalRows * cellHeight
    val maxScroll = max(0f, surfaceHeight - paint.contentHeight + reserved)
    if (maxScroll <= 0f) return CaretBand(0f, 0f)
    val pinnedTop = paint.contentBottom - surfaceHeight + cursorIndex * cellHeight
    // Whichever of the two floors is the higher, because a follower may rest below neither: below
    // the caret's it is typing off the top of the screen, and below the content's it is reading
    // blank tail. They coincide on the ordinary shell pane whose grid the output has filled, which
    // is why one of them served for as long as it did.
    val floor = max(
        paint.insetTop - pinnedTop,
        contentFloor(paint, totalRows, contentIndex, cellHeight, reserved),
    ).coerceIn(0f, maxScroll)
    return CaretBand(floor, (paint.contentBottom - cellHeight - pinnedTop).coerceIn(floor, maxScroll))
}

private const val READABLE_SP = 15f
private const val CLOSE_UP_SP = 22f

// Follow-cursor only nudges horizontally: the live viewport is already pinned to the bottom
// unless the operator has scrolled away, and scrolling away is a deliberate act to preserve.
//
// `null` is "the caret is already on screen, so there is nothing to do" — which is not the same
// answer as "leave the pan where it is", and telling them apart is what lets a hand-made pan give
// the axis back. Returning the unchanged `panX` for both is how a drag came to be undone by every
// frame that moved the caret.
fun followCursorPan(
    panX: Float,
    minPanX: Float,
    cursorCol: Int,
    cellWidth: Float,
    viewWidth: Float,
): Float? {
    if (minPanX >= 0f) return 0f
    val margin = cellWidth * 4f
    val left = cursorCol * cellWidth
    val right = left + cellWidth
    val target = when {
        left + panX < margin -> margin - left
        right + panX > viewWidth - margin -> viewWidth - margin - right
        else -> return null
    }
    return target.coerceIn(minPanX, 0f)
}
