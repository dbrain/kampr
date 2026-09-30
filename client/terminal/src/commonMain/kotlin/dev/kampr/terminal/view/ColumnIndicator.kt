package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.absolutePadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.layout
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edgeTop
import kotlin.math.floor
import kotlin.math.min

data class ColumnWindow(
    val firstCol: Int,
    val lastCol: Int,
    val cols: Int,
) {
    val columnsOff: Boolean get() = firstCol > 0 || lastCol < cols
}

fun columnWindow(panX: Float, paintWidth: Float, cols: Int, cellWidth: Float): ColumnWindow {
    val clampedPan = panX.coerceIn(min(0f, paintWidth - cols * cellWidth), 0f)
    val firstCol = floor(-clampedPan / cellWidth).toInt().coerceIn(0, cols)
    val lastCol = min(cols, firstCol + (paintWidth / cellWidth).toInt() + 1)
    return ColumnWindow(firstCol, lastCol, cols)
}

@Composable
fun ColumnIndicator(
    window: ColumnWindow,
    onOpen: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    val shape = RoundedCornerShape(tokens.radii.pill)
    val cols = window.cols.coerceAtLeast(1)
    val start = (window.firstCol.toFloat() / cols).coerceIn(0f, 1f)
    val span = ((window.lastCol - window.firstCol).toFloat() / cols).coerceIn(0.02f, 1f)
    val spoken = "Showing columns ${window.firstCol + 1} to ${window.lastCol} of ${window.cols}. Opens the zoom sheet."
    Row(
        modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeTop()
            .action(spoken, onOpen)
            .absolutePadding(left = 12.dp + safe.left, top = 5.dp, right = 12.dp + safe.right, bottom = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
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
            "col ${window.firstCol + 1}–${window.lastCol} of ${window.cols}",
            tokens.type.metaSmall,
            tokens.color.mute,
        )
    }
}
