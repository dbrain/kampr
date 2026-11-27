package dev.kampr.conversation

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText

// A capped box cut on a line boundary reads as the end of what it holds. The fade and the word say
// the rest is below, and go once the reader has scrolled to it.
@Composable
internal fun MoreBelow(scroll: ScrollState, ground: Color, modifier: Modifier = Modifier) {
    if (!scroll.canScrollForward) return
    val tokens = Kampr.tokens
    Box(
        modifier
            .fillMaxWidth()
            .height(34.dp)
            .background(Brush.verticalGradient(listOf(ground.copy(alpha = 0f), ground))),
    ) {
        KText(
            "more below ↓",
            tokens.type.micro,
            tokens.color.dim,
            Modifier.align(Alignment.BottomEnd).padding(end = 10.dp, bottom = 4.dp),
        )
    }
}
