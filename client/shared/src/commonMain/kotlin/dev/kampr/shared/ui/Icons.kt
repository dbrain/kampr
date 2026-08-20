package dev.kampr.shared.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Fill
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.unit.Dp

@Immutable
sealed interface Glyph {
    data class Trace(val d: String, val filled: Boolean = false) : Glyph
    data class Round(val cx: Float, val cy: Float, val r: Float, val filled: Boolean = false) : Glyph
    data class Frame(val x: Float, val y: Float, val w: Float, val h: Float, val r: Float) : Glyph
}

@Immutable
data class Icon(val viewport: Float, val stroke: Float, val parts: List<Glyph>)

private fun icon(viewport: Float, stroke: Float, vararg parts: Glyph) = Icon(viewport, stroke, parts.toList())

private fun d(path: String, filled: Boolean = false) = Glyph.Trace(path, filled)

object KamprIcons {
    val blockedAgent = icon(20f, 1.5f, d("M10 2v16M2 10h16M4.4 4.4l11.2 11.2M15.6 4.4 4.4 15.6"))
    val workingClock = icon(20f, 1.5f, Glyph.Round(10f, 10f, 7.5f), d("M10 5.5V10l3 2"))
    val shell = icon(20f, 1.5f, Glyph.Frame(2f, 3.5f, 16f, 13f, 2f), d("M5.5 8 8 10.2 5.5 12.4M10.5 12.6h4"))
    val done = icon(20f, 1.7f, d("M4 10.5 8.2 14.5 16 5.5"))
    val chevronRight = icon(14f, 1.8f, d("M4.5 2.5 9 7l-4.5 4.5"))
    val chevronLeft = icon(16f, 1.7f, d("M10 3 5 8l5 5"))
    val lock = icon(18f, 1.6f, Glyph.Frame(3f, 7.6f, 12f, 8f, 1.8f), d("M5.6 7.6V5.2a3.4 3.4 0 0 1 6.8 0v2.4"))
    val lockSmall = icon(14f, 1.6f, Glyph.Frame(3.4f, 6f, 7.2f, 6f, 1.4f), d("M4.8 6V4.2a2.2 2.2 0 0 1 4.4 0V6"))
    val gear = icon(
        16f, 1.6f, Glyph.Round(8f, 8f, 2f),
        d("M8 1.6v1.8M8 12.6v1.8M1.6 8h1.8M12.6 8h1.8M3.4 3.4l1.3 1.3M11.3 11.3l1.3 1.3M12.6 3.4l-1.3 1.3M4.7 11.3l-1.3 1.3"),
    )
    val zoom = icon(14f, 1.6f, Glyph.Round(6f, 6f, 4.4f), d("M9.4 9.4 12.6 12.6M4.2 6h3.6M6 4.2v3.6"))
    val herd = icon(20f, 1.7f, Glyph.Frame(2f, 3f, 16f, 5f, 1.5f), Glyph.Frame(2f, 12f, 16f, 5f, 1.5f))
    val pane = icon(20f, 1.7f, Glyph.Frame(2f, 2.5f, 16f, 15f, 2.5f), d("M5.5 8 8 10.2 5.5 12.4"))
    val nodes = icon(
        20f, 1.7f, Glyph.Round(10f, 4.5f, 2.5f), Glyph.Round(4.5f, 15.5f, 2.5f), Glyph.Round(15.5f, 15.5f, 2.5f),
        d("M10 7v3.4M8.4 12.4 6.6 13.6M11.6 12.4l1.8 1.2"),
    )
    val globe = icon(
        18f, 1.6f, Glyph.Round(9f, 9f, 7.2f),
        d("M1.8 9h14.4M9 1.8c1.9 2 2.9 4.5 2.9 7.2S10.9 15.2 9 16.2C7.1 15.2 6.1 11.7 6.1 9S7.1 3.8 9 1.8z"),
    )
    val warning = icon(16f, 1.7f, d("M8 2 15 14H1z"), d("M8 6.4v3.1M8 11.4v.5"))
    val tool = icon(16f, 1.6f, Glyph.Frame(1f, 2.5f, 14f, 11f, 2f), d("M4 6.4 6.2 8.2 4 10M8.4 10.4h3.4"))
    val workspace = icon(18f, 1.7f, Glyph.Frame(1.5f, 2.5f, 15f, 13f, 2f), d("M1.5 6.5h15"))
    val tab = icon(18f, 1.7f, Glyph.Frame(1.5f, 2.5f, 15f, 13f, 2f), d("M1.5 6.5h15M6.5 2.5v4"))
    val split = icon(18f, 1.7f, Glyph.Frame(1.5f, 2.5f, 15f, 13f, 2f), d("M9 2.5v13"))
    val branch = icon(
        18f, 1.7f, Glyph.Round(5f, 4f, 2.2f), Glyph.Round(5f, 14f, 2.2f), Glyph.Round(13f, 9f, 2.2f),
        d("M5 6.2v5.6M7.2 4h1.6A2.2 2.2 0 0 1 11 6.2v.9"),
    )
    val plus = icon(16f, 1.8f, d("M8 2.2v11.6M2.2 8h11.6"))
    val cross = icon(20f, 1.9f, d("M5 5l10 10M15 5 5 15"))
    val pencil = icon(16f, 1.6f, d("M10.6 2.6 13.4 5.4 5.4 13.4H2.6v-2.8z"))
    val ellipsis = icon(
        16f, 1.6f,
        Glyph.Round(3.2f, 8f, 1.1f, filled = true),
        Glyph.Round(8f, 8f, 1.1f, filled = true),
        Glyph.Round(12.8f, 8f, 1.1f, filled = true),
    )
}

@Composable
fun IconGlyph(icon: Icon, size: Dp, tint: Color, modifier: Modifier = Modifier) {
    val paths = remember(icon) {
        icon.parts.filterIsInstance<Glyph.Trace>().associateWith { PathParser().parsePathString(it.d).toPath() }
    }
    Canvas(modifier.size(size)) {
        val factor = this.size.minDimension / icon.viewport
        scale(factor, pivot = Offset.Zero) {
            val stroke = Stroke(width = icon.stroke)
            for (part in icon.parts) {
                when (part) {
                    is Glyph.Trace -> drawPath(
                        path = paths.getValue(part),
                        color = tint,
                        style = if (part.filled) Fill else stroke,
                    )
                    is Glyph.Round -> drawCircle(
                        color = tint,
                        radius = part.r,
                        center = Offset(part.cx, part.cy),
                        style = if (part.filled) Fill else stroke,
                    )
                    is Glyph.Frame -> drawRoundRect(
                        color = tint,
                        topLeft = Offset(part.x, part.y),
                        size = Size(part.w, part.h),
                        cornerRadius = CornerRadius(part.r, part.r),
                        style = stroke,
                    )
                }
            }
        }
    }
}
