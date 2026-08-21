package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.named

val HISTORY_EDGE_DP = 18.dp

// Where the record stops. It rides at the top of the scrollable surface rather than in the chrome,
// so an intact ring says its piece only to someone who scrolled all the way back to look.
@Composable
fun HistoryEdgeMark(label: String, spoken: String, broken: Boolean, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val ink = if (broken) tokens.color.working else tokens.color.mute
    Row(
        modifier.fillMaxWidth().height(HISTORY_EDGE_DP).named(spoken).padding(horizontal = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        KText(label, tokens.type.metaSmall, ink)
        Box(Modifier.weight(1f).height(1.dp).background(ink.copy(alpha = 0.5f)))
    }
}
