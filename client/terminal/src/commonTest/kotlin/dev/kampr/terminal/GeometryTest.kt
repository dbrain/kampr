package dev.kampr.terminal

import dev.kampr.terminal.view.PaintRect
import dev.kampr.terminal.view.defaultZoom
import dev.kampr.terminal.view.followCursorPan
import dev.kampr.terminal.view.initialScroll
import dev.kampr.terminal.view.terminalGeometry
import dev.kampr.terminal.view.TerminalViewState
import dev.kampr.terminal.view.zoomPresets
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val CELL_W = 8f
private const val CELL_H = 17f

private fun phone(insetTop: Float = 108f, insetBottom: Float = 130f) =
    PaintRect(width = 390f, height = 844f, insetTop = insetTop, insetBottom = insetBottom)

private fun close(a: Float, b: Float, tolerance: Float = 0.01f) =
    assertTrue(abs(a - b) < tolerance, "expected $b, got $a")

class GeometryTest {
    @Test
    fun defaultZoomFillsAtLeastOneAxis() {
        val paint = phone()
        val zoom = defaultZoom(paint, cols = 94, liveRows = 40, historyRows = 0, CELL_W, CELL_H)
        val fitWidth = paint.width / (94 * CELL_W)
        val fitHeight = paint.height / (40 * CELL_H)
        close(zoom, maxOf(fitWidth, fitHeight))
        assertTrue(zoom > minOf(fitWidth, fitHeight), "min() is what letterboxes")
    }

    @Test
    fun fillIsComputedAgainstThePaintRectangleNotTheInsetOne() {
        val wide = phone(insetTop = 0f, insetBottom = 0f)
        val chromed = phone()
        val a = defaultZoom(wide, 94, 40, 0, CELL_W, CELL_H)
        val b = defaultZoom(chromed, 94, 40, 0, CELL_W, CELL_H)
        assertEquals(a, b, "insetting the fill reintroduces the letterbox the insets exist to avoid")
    }

    @Test
    fun theLastLiveRowSettlesClearOfTheChrome() {
        val paint = phone()
        val geometry = terminalGeometry(paint, 94, 40, CELL_W, CELL_H, panX = 0f, scrollY = 0f)
        val lastRowBottom = geometry.originY + 40 * CELL_H
        close(lastRowBottom, paint.contentBottom)
        assertTrue(geometry.pinned)
    }

    @Test
    fun historyAndTheLiveGridShareOneScrollRange() {
        val paint = phone()
        val geometry = terminalGeometry(paint, 94, 40 + 300, CELL_W, CELL_H, 0f, 0f)
        close(geometry.surfaceHeight, 340 * CELL_H)
        close(geometry.maxScroll, 340 * CELL_H - paint.contentHeight)
        val top = terminalGeometry(paint, 94, 340, CELL_W, CELL_H, 0f, geometry.maxScroll)
        close(top.originY, paint.insetTop)
    }

    @Test
    fun scrollIsClampedAtBothEnds() {
        val paint = phone()
        val past = terminalGeometry(paint, 94, 340, CELL_W, CELL_H, 0f, 1e9f)
        close(past.scrollY, past.maxScroll)
        val before = terminalGeometry(paint, 94, 340, CELL_W, CELL_H, 0f, -500f)
        close(before.scrollY, 0f)
    }

    @Test
    fun aGridNarrowerThanTheViewportDoesNotPan() {
        val paint = phone()
        val geometry = terminalGeometry(paint, 20, 40, CELL_W, CELL_H, panX = -80f, scrollY = 0f)
        close(geometry.panX, 0f)
        close(geometry.minPanX, 0f)
    }

    @Test
    fun followCursorNudgesOnlyWhenTheCaretLeavesTheWindow() {
        val minPan = 390f - 94 * CELL_W
        val inside = followCursorPan(0f, minPan, cursorCol = 10, CELL_W, viewWidth = 390f)
        close(inside, 0f)
        val offRight = followCursorPan(0f, minPan, cursorCol = 80, CELL_W, viewWidth = 390f)
        assertTrue(offRight < 0f && offRight >= minPan)
        val visible = 80 * CELL_W + offRight
        assertTrue(visible in 0f..390f, "the caret must land inside the viewport")
    }

    @Test
    fun aPaneWithHistoryFillsWidthBecauseHistoryFillsTheHeight() {
        val paint = phone()
        val zoom = defaultZoom(paint, cols = 94, liveRows = 40, historyRows = 1200, CELL_W, CELL_H)
        close(zoom, paint.width / (94 * CELL_W))
        val surfaceHeight = 1240 * CELL_H * zoom
        assertTrue(surfaceHeight > paint.height, "history has to cover the height it claims to fill")
    }

    @Test
    fun aPaneOpensOnTheCaretWhenItWouldOtherwiseOpenOnBlankTail() {
        val paint = phone()
        val pinnedOnly = initialScroll(paint, totalRows = 34, cursorIndex = 33, cellHeight = 21f)
        close(pinnedOnly, 0f)
        val caretHigh = initialScroll(paint, totalRows = 34, cursorIndex = 4, cellHeight = 21f)
        assertTrue(caretHigh > 0f, "a caret above the fold has to pull the surface down")
        val geometry = terminalGeometry(paint, 94, 34, CELL_W, 21f, 0f, caretHigh)
        val caretTop = geometry.originY + 4 * 21f
        assertTrue(caretTop >= paint.insetTop && caretTop <= paint.contentBottom)
    }

    @Test
    fun presetsAreOrderedAndFitWidthActuallyFits() {
        val presets = zoomPresets(390f, 94, CELL_W)
        close(presets.fitWidth * 94 * CELL_W, 390f)
        assertTrue(presets.readable < presets.closeUp)
        assertTrue(presets.minimum < presets.fitWidth)
        assertTrue(presets.maximum > presets.closeUp)
    }
    // Picking a preset is the same move as finishing a pinch, and only one of the two used to
    // carry the viewport with it: pan and scroll are distances across the surface, so they scale
    // with the cell or the viewport lands on a different row than the one the operator was reading.
    @Test
    fun pickingAPresetKeepsTheRowTheOperatorWasReading() {
        val presets = zoomPresets(390f, 94, CELL_W)
        val view = TerminalViewState()
        view.setZoom(0.7f, presets)
        view.scrollY = 400f
        view.panX = -120f
        view.setZoom(1.4f, presets)
        close(view.scrollY, 800f)
        close(view.panX, -240f)
    }

    // The other half of the same move. `adoptDefault` re-derives the zoom every time history or
    // the real geometry lands, and it runs only while `chosen` is false — which is precisely the
    // state of a reader who has scrolled but never pinched.
    @Test
    fun reDerivingTheDefaultZoomKeepsTheRowTheOperatorWasReading() {
        val view = TerminalViewState()
        view.adoptDefault(0.7f)
        view.scrollY = 400f
        view.panX = -120f
        view.adoptDefault(1.4f)
        close(view.scrollY, 800f)
        close(view.panX, -240f)
    }

    // scrollY is a distance from the bottom of the surface, and rows leaving the live grid extend
    // that bottom — so a reader parked in history has to be carried by exactly what arrived, or
    // the row under their eye slides away. SurfaceRows.fromTop is the anchor review already uses.
    @Test
    fun historyArrivingUnderneathDoesNotMoveTheRowTheReaderIsOn() {
        val paint = phone()
        val view = TerminalViewState()
        view.maxScroll = 10_000f
        view.scrollY = 600f
        val parked = 10

        val before = terminalGeometry(paint, 94, 100, CELL_W, CELL_H, 0f, view.scrollY)
        val was = before.originY + parked * CELL_H

        view.carryHistory(5, CELL_H)

        val after = terminalGeometry(paint, 94, 105, CELL_W, CELL_H, 0f, view.scrollY)
        close(after.originY + parked * CELL_H, was)
    }

    @Test
    fun aReaderPinnedToTheBottomStaysPinnedWhenHistoryArrives() {
        val view = TerminalViewState()
        view.maxScroll = 10_000f
        view.scrollY = 0f
        view.carryHistory(5, CELL_H)
        close(view.scrollY, 0f)
    }
}
