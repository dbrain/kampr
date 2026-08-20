package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.layout
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.edge
import dev.kampr.terminal.render.Selection
import kotlin.math.roundToInt

private val HANDLE = 22.dp

private fun Modifier.atPixels(x: Float, y: Float) = layout { measurable, constraints ->
    val placeable = measurable.measure(constraints)
    layout(placeable.width, placeable.height) {
        placeable.place(x.roundToInt(), y.roundToInt())
    }
}

@Composable
fun SelectionLayer(
    selection: Selection,
    originX: Float,
    originY: Float,
    cellWidth: Float,
    cellHeight: Float,
    accent: Color,
    onAnchor: (Offset) -> Unit,
    onHead: (Offset) -> Unit,
    onCopy: () -> Unit,
    onBlock: () -> Unit,
    block: Boolean,
) {
    val tokens = Kampr.tokens
    val start = selection.start
    val end = selection.end
    val startX = originX + start.col * cellWidth
    val startY = originY + start.row * cellHeight
    val endX = originX + (end.col + 1) * cellWidth
    val endY = originY + (end.row + 1) * cellHeight

    Handle(startX, startY, accent, onAnchor)
    Handle(endX, endY - cellHeight, accent, onHead)

    val pillY = (startY - 46f).coerceAtLeast(4f)
    Row(
        Modifier
            .atPixels(startX.coerceAtLeast(4f), pillY)
            .background(tokens.color.raise, RoundedCornerShape(tokens.radii.md))
            .edge(tokens.card, RoundedCornerShape(tokens.radii.md)),
        horizontalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Box(Modifier.clickable(onClick = onCopy).padding(horizontal = 14.dp, vertical = 9.dp)) {
            KText("Copy", tokens.type.buttonSmall, tokens.color.text)
        }
        Box(Modifier.clickable(onClick = onBlock).padding(horizontal = 14.dp, vertical = 9.dp)) {
            KText(if (block) "Linear" else "Block", tokens.type.buttonSmall, tokens.color.dim)
        }
    }
}

@Composable
private fun Handle(x: Float, y: Float, accent: Color, onDrag: (Offset) -> Unit) {
    Box(
        Modifier
            .atPixels(x - 22f, y - 6f)
            .size(HANDLE)
            .background(accent, RoundedCornerShape(HANDLE))
            .pointerInput(x, y) {
                var at = Offset(x, y)
                detectDragGestures(
                    onDragStart = { at = Offset(x, y) },
                ) { _, delta ->
                    at += delta
                    onDrag(at)
                }
            },
    )
}
