package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.layout
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edge

data class ColumnWindow(
    val firstCol: Int,
    val lastCol: Int,
    val cols: Int,
    val rowsBack: Int,
)

@Composable
fun ColumnIndicator(
    window: ColumnWindow,
    reviewing: Boolean,
    onOpen: () -> Unit,
    onReview: () -> Unit,
    modifier: Modifier = Modifier,
    attachTo: String? = null,
    onAttach: (() -> Unit)? = null,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.pill)
    val cols = window.cols.coerceAtLeast(1)
    val start = (window.firstCol.toFloat() / cols).coerceIn(0f, 1f)
    val span = ((window.lastCol - window.firstCol).toFloat() / cols).coerceIn(0.02f, 1f)
    val trailer = if (window.rowsBack > 0) " · ${window.rowsBack} rows back" else ""

    val spoken = "Showing columns ${window.firstCol + 1} to ${window.lastCol} of ${window.cols}" +
        (if (window.rowsBack > 0) ", ${window.rowsBack} rows back" else "") + ". Opens the zoom sheet."
    Row(
        modifier
            .fillMaxWidth()
            .background(tokens.color.surface2)
            .padding(start = 12.dp, top = 4.dp, end = 8.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        // The bar and the review button are two controls, not one: merging them would leave the
        // way into review reachable only by opening a sheet about zoom.
        Row(
            Modifier.weight(1f).action(spoken, onOpen),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(9.dp),
        ) {
            Box(
                Modifier
                    .weight(1f)
                    .height(3.dp)
                    .background(tokens.color.raise, shape),
            ) {
                Box(
                    Modifier
                        .fillMaxWidth(span)
                        .height(3.dp)
                        .layout { measurable, constraints ->
                            val placeable = measurable.measure(constraints)
                            layout(placeable.width, placeable.height) {
                                placeable.place((constraints.maxWidth * start).toInt(), 0)
                            }
                        }
                        .background(tokens.color.dim, shape),
                )
            }
            KText(
                "col ${window.firstCol + 1}–${window.lastCol} of ${window.cols}$trailer",
                tokens.type.metaSmall,
                tokens.color.mute,
            )
        }

        // An agent over ssh reads a local path perfectly well; it is the terminal's own
        // image-paste protocol that dies. So this hands the bytes to the node, which writes them
        // beside the pane and types the path in. Absent where there is no picker to raise, and
        // absent on a device that may not type — a paste is typing.
        if (onAttach != null) {
            ChromePill("attach", "Attach a file for ${attachTo ?: "this pane"}", onAttach)
        }

        // The strip that review puts up carries its own way out, and two controls with the same
        // name is a worse thing to meet with a screen reader than one control that goes away.
        if (!reviewing) ChromePill("review", "Review this pane row by row", onReview)
    }
}

@Composable
private fun ChromePill(word: String, label: String, onClick: () -> Unit) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.pill)
    Row(
        Modifier
            .defaultMinSize(minHeight = 26.dp)
            .background(tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .action(label, onClick, shape)
            .padding(horizontal = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        KText(word, tokens.type.metaSmall, tokens.color.dim)
    }
}
