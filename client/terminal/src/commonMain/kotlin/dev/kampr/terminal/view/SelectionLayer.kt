package dev.kampr.terminal.view

import androidx.compose.foundation.background
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
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.touchable
import dev.kampr.terminal.render.Selection
import kotlin.math.roundToInt

private val HANDLE = 22.dp

private const val MARGIN = 4f

// Placed at a cell's own pixel, and then held inside the box it is drawn in. A selection near the
// right-hand edge puts its start column most of a screen across, and the pill is as wide as three
// buttons: unclamped it hung 182 dp of itself past a 411 dp phone, so the one affordance a long
// press exists to offer was off the screen exactly where a right thumb lands. The handles are held
// by the same arithmetic — one that cannot be touched cannot be dragged.
private fun Modifier.atPixels(x: Float, y: Float) = layout { measurable, constraints ->
    val placeable = measurable.measure(constraints)
    fun hold(at: Float, size: Int, room: Int, bounded: Boolean): Int {
        if (!bounded) return at.roundToInt()
        return at.coerceIn(MARGIN, (room - size - MARGIN).coerceAtLeast(MARGIN)).roundToInt()
    }
    val left = hold(x, placeable.width, constraints.maxWidth, constraints.hasBoundedWidth)
    val top = hold(y, placeable.height, constraints.maxHeight, constraints.hasBoundedHeight)
    layout(placeable.width, placeable.height) {
        placeable.place(left, top)
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
    onPaste: (() -> Unit)?,
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

    Handle(startX, startY, accent, "Selection start handle", onAnchor)
    Handle(endX, endY - cellHeight, accent, "Selection end handle", onHead)

    Row(
        Modifier
            .atPixels(startX, startY - 46f)
            .background(tokens.color.raise, RoundedCornerShape(tokens.radii.md))
            .edge(tokens.card, RoundedCornerShape(tokens.radii.md)),
        horizontalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Box(
            Modifier
                .touchable()
                .action("Copy the selection", onCopy)
                .padding(horizontal = 14.dp, vertical = 9.dp),
            contentAlignment = androidx.compose.ui.Alignment.Center,
        ) {
            KText("Copy", tokens.type.buttonSmall, tokens.color.text)
        }
        // The terminal's context menu is this pill, and paste is what a long press on a terminal is
        // for. Absent rather than present-and-refusing on a read-only device, like every other
        // write affordance. It is not about the selection — a terminal has nothing to replace —
        // so it sits beside Copy rather than acting on what Copy would take.
        if (onPaste != null) {
            Box(
                Modifier
                    .touchable()
                    .action("Paste the clipboard into the pane", onPaste)
                    .padding(horizontal = 14.dp, vertical = 9.dp),
                contentAlignment = androidx.compose.ui.Alignment.Center,
            ) {
                KText("Paste", tokens.type.buttonSmall, tokens.color.text)
            }
        }
        Box(
            Modifier
                .touchable()
                .action(
                    if (block) "Select by line instead of by column" else "Select by column instead of by line",
                    onBlock,
                )
                .padding(horizontal = 14.dp, vertical = 9.dp),
            contentAlignment = androidx.compose.ui.Alignment.Center,
        ) {
            KText(if (block) "Linear" else "Block", tokens.type.buttonSmall, tokens.color.dim)
        }
    }
}

@Composable
private fun Handle(x: Float, y: Float, accent: Color, label: String, onDrag: (Offset) -> Unit) {
    Box(
        Modifier
            .atPixels(x - 22f, y - 6f)
            .named(label)
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
