package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
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
) {
    val clipped: Boolean get() = firstCol > 0 || lastCol < cols || rowsBack > 0
}

@Composable
fun ColumnIndicator(
    window: ColumnWindow,
    onOpen: () -> Unit,
    modifier: Modifier = Modifier,
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
            .background(tokens.color.bar, shape)
            .edge(tokens.card, shape)
            .action(spoken, onOpen, shape)
            .padding(horizontal = 10.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(
            Modifier
                .width(48.dp)
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
}
