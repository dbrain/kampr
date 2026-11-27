package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.PaneInfo

// docs/24: a program's `error`, which `agent_status` has no word for and leaves `idle`.
fun programFailure(pane: PaneInfo): String? = if (pane.programError) listOfNotNull("Failed", pane.programMsg).joinToString(": ") else null

// A pane's status mark with a failed program's warning on its corner, as the pane header corners
// its own warnings. A row's long press is its menu, so a finger hears the words in the row's label
// and a pointer gets them on hover.
@Composable
fun ProgramMarked(pane: PaneInfo, badge: Boolean = false, mark: @Composable () -> Unit) {
    val failure = programFailure(pane)
    if (failure == null) {
        mark()
        return
    }
    val manage = LocalManage.current
    Box(Modifier.named(failure), contentAlignment = Alignment.Center) {
        mark()
        IconGlyph(
            KamprIcons.warning,
            if (badge) 13.dp else 10.dp,
            Kampr.tokens.color.blocked,
            if (badge) Modifier.align(Alignment.TopEnd).offset(4.dp, (-4).dp) else Modifier.offset(6.dp, (-7).dp),
        )
        Hint(failure, byTouch = !manage.enabled)
    }
}

@Composable
fun ProgramFailureBadge(pane: PaneInfo) {
    val tokens = Kampr.tokens
    val failure = programFailure(pane) ?: return
    Box(Modifier.announce(listOfNotNull("The program failed", pane.programMsg).joinToString(": "))) {
        StatusBadge("Failed", tokens.color.blocked, tokens.color.blockedBg, icon = KamprIcons.warning, outlined = true)
        if (!LocalStatusUnseen.current) Hint(failure)
    }
}

// The records list's way in where statuses are drawn as badges. Where they are composed unseen
// under one mark, the mark's own long press opens the list instead, and an invisible target here
// would take the mark's tap.
@Composable
fun ProgramsBadge(count: Int, onOpen: () -> Unit) {
    val tokens = Kampr.tokens
    val words = "$count programs"
    if (LocalStatusUnseen.current) {
        Box(Modifier.named(words))
        return
    }
    Box(Modifier.action("$words, show them", onOpen, RoundedCornerShape(tokens.radii.pill))) {
        StatusBadge(words, tokens.color.dim, tokens.color.surface, outlined = true)
    }
}

// How far a working program says it is, as a hairline along the bottom of whatever it is drawn on,
// so a row or a header that gains one moves nothing.
fun Modifier.progressEdge(progress: Int?, tone: Color, inset: Dp = 0.dp): Modifier {
    if (progress == null) return this
    val fraction = progress.coerceIn(0, 100) / 100f
    return semantics { progressBarRangeInfo = ProgressBarRangeInfo(fraction, 0f..1f) }
        .drawWithContent {
            drawContent()
            val thick = 2.dp.toPx()
            val left = inset.toPx()
            val width = (size.width - 2 * left).coerceAtLeast(0f)
            val top = size.height - thick
            drawRect(tone.copy(alpha = 0.25f), Offset(left, top), Size(width, thick))
            drawRect(tone, Offset(left, top), Size(width * fraction, thick))
        }
}
