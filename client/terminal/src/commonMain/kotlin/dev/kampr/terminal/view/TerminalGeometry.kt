package dev.kampr.terminal.view

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

// What a pane opens at, and it is a constant on purpose.
//
// It used to be a fill: the largest zoom that put the grid — **and its scrollback** — inside the
// viewport, floored at 1.0x wherever the window was wide enough and capped at 1.0x on a desk. The
// operator, who had it produce both ends of its range: *"its resulting in sometimes being zoomed at
// 0.4x which is ridiculously small, other times 3.7x which is ridiculously large. 1.0x seems to be
// the best on all devices."*
//
// **And a fill that counts the ring cannot be derived twice.** The rows it had to fill were the
// live grid plus whatever history the pane held, so the same pane answered 1.067x with no ring and
// **0.560x** once the node's first scrollback frame landed — measured on a real pane, about two
// seconds after `df -h` first scrolled the grid. Nothing had moved: the cell halved under a reader
// who had chosen nothing, which is the jump the operator reported as the terminal not sticking to
// the bottom. A constant has no second derivation and nothing to re-derive it from.
//
// The fit ladder still exists and is still one press away — `ZoomPresets` carries fit-width — but
// it is somewhere the operator goes rather than somewhere a pane starts.
const val DEFAULT_ZOOM = 1f

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
// **Clamped to the top of the pane's own grid, which is the room it is allowed to make.** The
// floor's job is to keep the end of the record on the screen; past the first row of the grid it is
// spending the operator's history to hide the pane's blank tail, and history is not blank — it is
// where the surface was scrolled *away from*. A full-screen program is what makes the difference
// visible, because leaving one moves the end of the record by most of a pane in a single frame
// (probe #502): every row written and the caret at the top while it holds the screen, five rows of
// forty when it gives it back. On the desk, where the pane is the size of the view and the whole
// grid is on the screen at the bottom of the surface, that swing hauled the viewport 19 rows into
// a `top` the operator had run before it and pinned them there — a hand's floor is this same
// number, so there was no scrolling back out. Where the grid genuinely overflows the rectangle the
// clamp does not bind and the floor is what it was: `aPaneWithNothingOnItButItsCaretCannotBeDragged`
// `IntoTheTail` is the case that was already this rule, arrived at by having no history to climb —
// there `maxScroll` *was* the top of the grid, which is why it read as a clamp on the travel.
//
// A grid that fits its rectangle answers zero, as it did before there was a floor of any kind, and
// the surface's own travel needs no clamp of its own: the grid is the tail of the surface, so the
// top of it is never above the top of what there is to scroll. The `reserved` rows the node is
// holding back are more of that surface, above the grid, and no part of this room.
fun contentFloor(
    paint: PaintRect,
    totalRows: Int,
    contentIndex: Int,
    cellHeight: Float,
    gridRows: Int,
): Float {
    val topOfTheGrid = max(0f, gridRows * cellHeight - paint.contentHeight)
    return ((totalRows - 1 - contentIndex) * cellHeight).coerceIn(0f, topOfTheGrid)
}

fun caretBand(
    paint: PaintRect,
    totalRows: Int,
    cursorIndex: Int,
    contentIndex: Int,
    cellHeight: Float,
    gridRows: Int,
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
        contentFloor(paint, totalRows, contentIndex, cellHeight, gridRows),
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
