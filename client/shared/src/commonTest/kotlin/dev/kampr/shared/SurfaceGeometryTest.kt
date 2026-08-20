package dev.kampr.shared

import dev.kampr.shared.model.surfaceGeometry
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private const val CELL_W = 7.8f
private const val CELL_H = 17f

private fun close(a: Float, b: Float) = abs(a - b) < 0.01f

class SurfaceGeometryTest {
    @Test
    fun neverLetterboxesAtAnyBreakpointOrPaneShape() {
        val viewports = listOf(1440f to 900f, 390f to 844f, 844f to 390f, 690f to 837f)
        val panes = listOf(74 to 30, 89 to 34, 94 to 24, 200 to 50, 40 to 12)
        for ((width, height) in viewports) {
            for ((cols, rows) in panes) {
                for (history in listOf(0, 1, 171, 1553, 100_000)) {
                    val geometry = surfaceGeometry(width, height, cols, rows, history, CELL_W, CELL_H)
                    assertFalse(
                        geometry.letterboxed,
                        "letterboxed at ${width}x$height for ${cols}x$rows history=$history",
                    )
                    assertTrue(geometry.surfaceHeight >= height - 0.01f)
                }
            }
        }
    }

    @Test
    fun zoomFillsTheLargerAxisRatherThanFittingInsideBoth() {
        val wideViewport = surfaceGeometry(1440f, 400f, 89, 34, 0, CELL_W, CELL_H)
        val fitWidth = 1440f / (89 * CELL_W)
        val fitHeight = 400f / (34 * CELL_H)
        assertTrue(fitHeight < fitWidth)
        assertTrue(close(wideViewport.zoom, fitWidth))

        val tallViewport = surfaceGeometry(690f, 837f, 89, 34, 0, CELL_W, CELL_H)
        assertTrue(tallViewport.zoom > 690f / (89 * CELL_W))
    }

    @Test
    fun liveViewportIsPinnedToTheBottomWithHistoryAbove() {
        val history = 1553
        val geometry = surfaceGeometry(390f, 844f, 94, 40, history, CELL_W, CELL_H)
        val rowHeight = CELL_H * geometry.zoom
        val liveBottom = geometry.originY + (history + 40) * rowHeight
        assertTrue(close(liveBottom, 844f), "live grid must end exactly at the viewport bottom")
        assertTrue(geometry.originY < 0f, "history must extend above the viewport")
        assertTrue(close(geometry.originY, -(history * rowHeight)), "history is contiguous with the grid")
    }

    @Test
    fun aPaneWithNoRingStillFillsTheSurface() {
        val geometry = surfaceGeometry(844f, 390f, 89, 34, 0, CELL_W, CELL_H)
        val rowHeight = CELL_H * geometry.zoom
        assertTrue(close(geometry.originY + 34 * rowHeight, 390f))
        assertTrue(geometry.originY <= 0f)
    }

    @Test
    fun deepHistoryIsNotCapped() {
        val shallow = surfaceGeometry(390f, 844f, 94, 40, 1_000, CELL_W, CELL_H)
        val deep = surfaceGeometry(390f, 844f, 94, 40, 1_000_000, CELL_W, CELL_H)
        assertEquals(shallow.zoom, deep.zoom)
        assertTrue(deep.surfaceHeight > shallow.surfaceHeight * 900)
    }

    @Test
    fun degenerateGeometryDoesNotDivideByZero() {
        val geometry = surfaceGeometry(390f, 844f, 0, 0, 0, CELL_W, CELL_H)
        assertEquals(1f, geometry.zoom)
        assertFalse(geometry.letterboxed)
    }
}
