package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.Hint
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LocalZoomGlyph
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.touchable
import dev.kampr.terminal.PaneSession

// Sits beside the view tabs and carries the same weight as them: a real button with a 44 dp touch
// target, not a chip bolted onto the end of a tab bar.
@Composable
fun ZoomButton(session: PaneSession, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    val open = session.view.sheetOpen
    // Rule 3's price for a hold: said wherever it is held, on the one control that opens the panel
    // its off switch is on. ADR 0013.
    val held = session.view.holdsPane
    val spoken = "Zoom, currently ${zoomLabel(session.view.displayZoom)}"
    val glyph = LocalZoomGlyph.current
    if (glyph && !held) return
    Box(
        modifier
            .defaultMinSize(minWidth = if (glyph) 36.dp else 68.dp)
            .background(if (open) tokens.color.accentSoft else tokens.color.surface, shape)
            .edge(tokens.card, shape)
            .touchable()
            .action(
                spoken,
                { session.view.sheetOpen = !open },
                shape,
                selected = open,
                state = listOfNotNull(
                    "sheet open".takeIf { open },
                    "holding this pane at this view's size — the desk sees it this shape too".takeIf { held },
                ).joinToString(", ").ifEmpty { null },
            ),
        contentAlignment = Alignment.CenterStart,
    ) {
        Row(
            Modifier.padding(horizontal = if (glyph) 10.dp else 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            IconGlyph(KamprIcons.zoom, 15.dp, if (open) tokens.color.accent else tokens.color.dim)
            if (!glyph) {
                KText(
                    zoomLabel(session.view.displayZoom),
                    tokens.type.key,
                    if (open) tokens.color.accent else tokens.color.dim,
                )
            }
            if (held) KText("held", tokens.type.key, tokens.color.working)
        }
        Hint(spoken)
    }
}
