package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.edgeTop
import dev.kampr.shared.ui.group
import dev.kampr.shared.ui.named
import dev.kampr.terminal.review.ReviewMove
import dev.kampr.terminal.review.ReviewState

private val REVIEW_TOUCH = 40.dp

// Every control here is a real button rather than a gesture, because the readers this exists for
// reach it with a double tap or with Tab and Return, and a gesture detector sees neither.
@Composable
fun ReviewStrip(
    review: ReviewState,
    total: Int,
    warning: String?,
    onMove: (ReviewMove) -> Unit,
    onLeave: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    Column(
        modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeTop()
            // With focus anywhere in the strip the arrow keys are review's, not the pane's — the
            // offscreen input that would otherwise swallow them is not the focused thing.
            .onPreviewKeyEvent { event ->
                if (event.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                val move = when (event.key) {
                    Key.DirectionUp -> ReviewMove.PreviousLine
                    Key.DirectionDown -> ReviewMove.NextLine
                    Key.DirectionLeft -> ReviewMove.PreviousWord
                    Key.DirectionRight -> ReviewMove.NextWord
                    Key.Escape -> null
                    else -> return@onPreviewKeyEvent false
                }
                if (move == null) onLeave() else onMove(move)
                true
            }
            .group(),
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 8.dp, vertical = 5.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            Control("◀", "Read the previous word") { onMove(ReviewMove.PreviousWord) }
            Control("▲", "Read the previous row") { onMove(ReviewMove.PreviousLine) }
            Control("▼", "Read the next row") { onMove(ReviewMove.NextLine) }
            Control("▶", "Read the next word") { onMove(ReviewMove.NextWord) }
            Control("again", "Read this row again") { onMove(ReviewMove.Reread) }
            Control("now", "Back to the live cursor") { onMove(ReviewMove.Now) }
            Control("done", "Leave review", accent = true, onClick = onLeave)
            // The warning is tint and speech here, not more words: it is already written out at
            // the top of the surface, in the grid's own description and in the zoom sheet.
            KText(
                "row ${review.row + 1} of $total",
                tokens.type.metaSmall,
                if (warning == null) tokens.color.mute else tokens.color.working,
                Modifier
                    .padding(start = 6.dp)
                    .named("Row ${review.row + 1} of $total" + (warning?.let { ". The $it" } ?: "")),
            )
        }

        // Two one-dp strips, nothing to look at. The first is the only thing that speaks while
        // review is on, and it speaks because the reader asked. The second fires once when the
        // pane writes to the row they are parked on, and never again until they read.
        Box(Modifier.fillMaxWidth().height(1.dp).announce(review.spoken))
        Box(Modifier.fillMaxWidth().height(1.dp).announce(review.notice))
    }
}

@Composable
private fun Control(glyph: String, label: String, accent: Boolean = false, onClick: () -> Unit) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Box(
        Modifier
            .defaultMinSize(minWidth = REVIEW_TOUCH, minHeight = REVIEW_TOUCH)
            .background(if (accent) tokens.color.accentSoft else tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .action(label, onClick, shape)
            .padding(horizontal = 9.dp),
        contentAlignment = Alignment.Center,
    ) {
        KText(
            glyph,
            if (glyph.length == 1) tokens.type.badge else tokens.type.key,
            if (accent) tokens.color.accent else tokens.color.text,
        )
    }
}
