package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
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
private val DOT = 10.dp

private const val MARGIN = 4f

// Placed at a cell's own pixel, and then held inside the box it is drawn in. A selection near the
// right-hand edge puts its start column most of a screen across, and the pill is as wide as three
// buttons: unclamped it hung 182 dp of itself past a 411 dp phone, so the one affordance a long
// press exists to offer was off the screen exactly where a right thumb lands. The handles are held
// by the same arithmetic — one that cannot be touched cannot be dragged.
internal fun Modifier.atPixels(x: Float, y: Float) = layout { measurable, constraints ->
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

    // The handles flank the selection: the dot's near edge is on the cell's edge, so the glyphs
    // they mark stay readable. The box around the dot is the drag target and keeps its 22 dp.
    Handle(startX - 5f, startY + cellHeight / 2f, accent, tokens.color.onAccent, "Selection start handle", onAnchor)
    Handle(endX + 5f, endY - cellHeight / 2f, accent, tokens.color.onAccent, "Selection end handle", onHead)

    // The pill sits below the selection, not above it: above is where the text the operator is
    // reading lives, and a three-button pill over it is the selection covering itself. Below is
    // the end of the record, and `atPixels` holds the pill inside the box when the selection is
    // low enough that there is no below.
    Row(
        Modifier
            .atPixels(startX, endY + 8f)
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
private fun Handle(
    cx: Float,
    cy: Float,
    accent: Color,
    outline: Color,
    label: String,
    onDrag: (Offset) -> Unit,
) {
    Box(
        Modifier
            .atPixels(cx - 11f, cy - 11f)
            .named(label)
            .size(HANDLE)
            .pointerInput(cx, cy) {
                var at = Offset(cx, cy)
                detectDragGestures(
                    onDragStart = { at = Offset(cx, cy) },
                ) { _, delta ->
                    at += delta
                    onDrag(at)
                }
            },
        contentAlignment = androidx.compose.ui.Alignment.Center,
    ) {
        Box(
            Modifier
                .size(DOT)
                .background(accent, CircleShape)
                .border(1.5.dp, outline, CircleShape),
        )
    }
}
