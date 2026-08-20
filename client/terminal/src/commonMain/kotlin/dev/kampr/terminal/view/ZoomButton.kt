package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.touchable
import dev.kampr.terminal.PaneSession

private fun label(zoom: Float): String {
    if (zoom <= 0f) return "fit"
    val tenths = (zoom * 10f + 0.5f).toInt()
    return "${tenths / 10}.${tenths % 10}×"
}

// Sits beside the view tabs and carries the same weight as them: a real button with a 44 dp touch
// target, not a chip bolted onto the end of a tab bar.
@Composable
fun ZoomButton(session: PaneSession, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    val open = session.view.sheetOpen
    Row(
        modifier
            .defaultMinSize(minWidth = 68.dp)
            .background(if (open) tokens.color.accentSoft else tokens.color.surface, shape)
            .edge(tokens.card, shape)
            .touchable()
            .action(
                "Zoom, currently ${label(session.view.displayZoom)}",
                { session.view.sheetOpen = !open },
                shape,
                selected = open,
                state = if (open) "sheet open" else null,
            )
            .padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        IconGlyph(KamprIcons.zoom, 15.dp, if (open) tokens.color.accent else tokens.color.dim)
        KText(
            label(session.view.displayZoom),
            tokens.type.key,
            if (open) tokens.color.accent else tokens.color.dim,
        )
    }
}
