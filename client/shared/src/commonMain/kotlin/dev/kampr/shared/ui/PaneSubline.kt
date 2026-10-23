package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.homeRelative
import dev.kampr.shared.model.nativeFacts
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.PaneInfo

// The facts ride the directory's line rather than the title's, so a 296 dp sidebar row gives none
// of the pane's name to them. Spoken through `paneSpoken`, as `WatchersTag` is.
@Composable
fun PaneSubline(pane: PaneInfo) {
    val tokens = Kampr.tokens
    val facts = nativeFacts(pane)
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        if (facts.isNotEmpty()) {
            KText(facts.joinToString(" · "), tokens.type.micro, if (pane.exited != null) tokens.color.dim else tokens.color.accent)
        }
        KText(homeRelative(pane.cwd ?: ""), tokens.type.meta, tokens.color.mute, Modifier.weight(1f, fill = false), overflow = ELIDE_MIDDLE)
    }
}
