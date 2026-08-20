package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.edge
import dev.kampr.terminal.render.Target
import dev.kampr.terminal.render.TargetKind

// A detected URL is not a declared one. Pane output is attacker-influenceable, so the target is
// shown and acted on deliberately rather than navigated to on touch (probes #36/#37).
@Composable
fun TargetStrip(
    target: Target,
    onAct: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Row(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(
            Modifier
                .weight(1f)
                .background(tokens.color.surface, shape)
                .edge(tokens.card, shape)
                .clickable(onClick = onDismiss)
                .padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            KText(
                when (target.kind) {
                    TargetKind.Link -> "link"
                    TargetKind.Url -> "detected"
                    TargetKind.Path -> "path"
                },
                tokens.type.metaSmall,
                tokens.color.mute,
            )
            KText(target.text, tokens.type.caption, tokens.color.text, Modifier.weight(1f))
        }
        Box(
            Modifier
                .background(tokens.color.accent, shape)
                .clickable(onClick = onAct)
                .padding(horizontal = 16.dp, vertical = 11.dp),
        ) {
            KText(
                if (target.kind == TargetKind.Path) "Copy" else "Open",
                tokens.type.buttonSmall,
                tokens.color.onAccent,
            )
        }
    }
}
