package dev.kampr.app

import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Rect
import android.util.DisplayMetrics
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs
import kotlin.math.max

// The drawable's own comment stated this constraint from the day it was written and nothing ever
// checked it, which is how a sheep shipped as a 12x9px blob in the bottom half of the badge. mdpi
// is the worst case: Android draws the small icon at 24dp, so every denser screen gets more pixels
// for the same geometry.
class NotificationIconGeometryTest {
    @Test
    fun theNotificationIconFillsItsKeylineAndIsCentred() {
        val ink = inkBoxAtMdpi(R.drawable.ic_kampr_notification)

        assertTrue(
            "artwork is $ink in a ${CANVAS}px badge — it does not reach the ${KEYLINE}dp keyline, " +
                "so it renders as a dot rather than a glyph",
            max(ink.width(), ink.height()) >= KEYLINE - SLACK,
        )
        assertTrue(
            "artwork is $ink in a ${CANVAS}px badge — margins ${ink.left}/${CANVAS - ink.right} " +
                "horizontally, ${ink.top}/${CANVAS - ink.bottom} vertically; it sits off centre",
            abs(ink.left - (CANVAS - ink.right)) <= SLACK &&
                abs(ink.top - (CANVAS - ink.bottom)) <= SLACK,
        )
    }

    private fun inkBoxAtMdpi(drawableId: Int): Rect {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val mdpi = context.createConfigurationContext(
            Configuration(context.resources.configuration).apply {
                densityDpi = DisplayMetrics.DENSITY_MEDIUM
            },
        )
        val drawable = requireNotNull(mdpi.getDrawable(drawableId))

        assertEquals(
            "the drawable no longer declares 24dp, so it is not the size Android draws",
            CANVAS,
            drawable.intrinsicWidth,
        )
        assertEquals(CANVAS, drawable.intrinsicHeight)

        val bitmap = Bitmap.createBitmap(CANVAS, CANVAS, Bitmap.Config.ARGB_8888)
        drawable.setBounds(0, 0, CANVAS, CANVAS)
        drawable.draw(Canvas(bitmap))

        val pixels = IntArray(CANVAS * CANVAS)
        bitmap.getPixels(pixels, 0, CANVAS, 0, 0, CANVAS, CANVAS)

        var left = CANVAS
        var top = CANVAS
        var right = -1
        var bottom = -1
        for (y in 0 until CANVAS) {
            for (x in 0 until CANVAS) {
                if (pixels[y * CANVAS + x] ushr 24 == 0) continue
                if (x < left) left = x
                if (x > right) right = x
                if (y < top) top = y
                if (y > bottom) bottom = y
            }
        }
        if (right < 0) throw AssertionError("the drawable rasterised to nothing at 24dp mdpi")
        return Rect(left, top, right + 1, bottom + 1)
    }

    private companion object {
        // 24dp at mdpi is 24px, and the 22dp keyline inside it is what Android's own icon
        // guidance reserves; SLACK is the granularity of a 24px raster, not a design allowance.
        const val CANVAS = 24
        const val KEYLINE = 22
        const val SLACK = 1
    }
}
