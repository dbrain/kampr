package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.touchable

@Composable
internal fun AnswerChip(
    label: String,
    lit: Boolean,
    enabled: Boolean,
    onPress: () -> Unit,
    modifier: Modifier = Modifier,
    selected: Boolean? = null,
    content: @Composable ColumnScope.(ink: Color) -> Unit,
) {
    val tokens = Kampr.tokens
    val chip = RoundedCornerShape(tokens.radii.sm)
    val shown = lit && enabled
    Column(
        modifier
            .background(if (shown) tokens.color.accent else tokens.color.raise, chip)
            .edge(if (shown) BorderSpec(0.dp, tokens.color.accent) else tokens.card, chip)
            .touchable(LocalTouch.current)
            .action(label, onPress, chip, enabled = enabled, selected = selected)
            .padding(horizontal = 12.dp, vertical = 9.dp),
        verticalArrangement = Arrangement.spacedBy(3.dp),
    ) {
        val ink = when {
            !enabled -> tokens.color.mute
            lit -> tokens.color.onAccent
            else -> tokens.color.text
        }
        content(ink)
    }
}
