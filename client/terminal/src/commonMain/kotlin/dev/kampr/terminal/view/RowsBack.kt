package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
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

@Composable
fun RowsBackChip(rowsBack: Int, onJump: () -> Unit, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.pill)
    Row(
        modifier
            .background(tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .touchable()
            .action("$rowsBack rows back. Jump to the live edge.", onJump, shape)
            .padding(horizontal = 12.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        KText("$rowsBack rows back", tokens.type.metaSmall, tokens.color.mute)
        KText("Jump to bottom", tokens.type.buttonSmall, tokens.color.text)
        IconGlyph(KamprIcons.chevronDown, 12.dp, tokens.color.text)
    }
}
