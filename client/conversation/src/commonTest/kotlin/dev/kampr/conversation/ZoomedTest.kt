package dev.kampr.conversation

import androidx.compose.ui.unit.IntSize
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val PANE = IntSize(400, 800)
private val FIT = Zoomed(1f, 0f, 0f)

class ZoomedTest {
    @Test
    fun aPictureCannotBeShrunkBelowTheViewportOrBlownUpWithoutEnd() {
        assertEquals(1f, zoomed(FIT, 0.2f, 0f, 0f, PANE).zoom)
        assertEquals(8f, zoomed(Zoomed(6f, 0f, 0f), 4f, 0f, 0f, PANE).zoom)
    }

    // The one that strands a picture: pan is bounded by half the width the zoom added, so the far
    // edge can be reached and nothing past it can.
    @Test
    fun panStopsWhereThePictureRunsOut() {
        val zoomedIn = zoomed(FIT, 2f, 0f, 0f, PANE)
        assertEquals(2f, zoomedIn.zoom)
        val shoved = zoomed(zoomedIn, 1f, 10_000f, 10_000f, PANE)
        assertEquals(200f, shoved.panX, "pan ran past half the width a 2x zoom added")
        assertEquals(400f, shoved.panY, "pan ran past half the height a 2x zoom added")
        val other = zoomed(zoomedIn, 1f, -10_000f, -10_000f, PANE)
        assertEquals(-200f, other.panX)
        assertEquals(-400f, other.panY)
    }

    // Pulled back to its own size it has nowhere left to go, and leaving it offset there is a
    // reader looking at a corner with the gesture that would fix it already spent.
    @Test
    fun aPictureBackAtItsOwnSizeIsBackInTheMiddle() {
        val corner = zoomed(zoomed(FIT, 4f, 0f, 0f, PANE), 1f, 10_000f, 10_000f, PANE)
        assertTrue(corner.panX > 0f && corner.panY > 0f, "the picture never left the middle to begin with")
        val out = zoomed(corner, 0.01f, 0f, 0f, PANE)
        assertEquals(Zoomed(1f, 0f, 0f).panX, out.panX, "a picture back at its own size kept an offset")
        assertEquals(Zoomed(1f, 0f, 0f).panY, out.panY)
    }

    // Zooming keeps the reader over the same part of the picture rather than sliding it back to
    // the middle, so the offset grows with the zoom that earned it.
    @Test
    fun theOffsetGrowsWithTheZoomRatherThanSurvivingItUnchanged() {
        val held = zoomed(zoomed(FIT, 2f, 0f, 0f, PANE), 1f, 100f, 0f, PANE)
        assertEquals(100f, held.panX)
        val closer = zoomed(held, 2f, 0f, 0f, PANE)
        assertEquals(4f, closer.zoom)
        assertEquals(200f, closer.panX, "the offset did not follow the zoom that doubled around it")
    }

    // A gesture that arrives before the viewport has been measured must not divide by it.
    @Test
    fun aGestureBeforeAnythingIsMeasuredGoesNowhere() {
        val early = zoomed(FIT, 2f, 500f, 500f, IntSize.Zero)
        assertEquals(0f, early.panX)
        assertEquals(0f, early.panY)
    }
}
