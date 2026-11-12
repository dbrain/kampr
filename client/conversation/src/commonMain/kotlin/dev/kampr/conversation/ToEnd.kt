package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.TOUCH
import dev.kampr.shared.ui.edge

@Composable
internal fun ToEnd(onClick: () -> Unit, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    GlyphTarget(
        ConversationIcons.toEnd,
        "Go to the end of the transcript",
        tokens.color.accent,
        onClick,
        modifier.background(tokens.color.surface, CircleShape).edge(tokens.card, CircleShape),
        target = TOUCH,
        glyph = 15.dp,
    )
}
