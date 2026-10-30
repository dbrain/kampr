package dev.kampr.conversation.files

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.Icon
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.TOUCH
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.touchable

@Composable
fun EntryRow(
    icon: Icon?,
    name: String,
    facts: String?,
    badge: String?,
    spoken: String,
    onLongClick: (() -> Unit)? = null,
    trailing: (@Composable RowScope.() -> Unit)? = null,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    val tokens = Kampr.tokens
    Row(modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Row(
            Modifier
                .weight(1f)
                .touchable(TOUCH)
                .action(spoken, onClick, onLongClick = onLongClick)
                .padding(horizontal = 4.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            icon?.let { IconGlyph(it, 16.dp, tokens.color.dim) }
            Column(Modifier.weight(1f)) {
                KText(name, tokens.type.meta, tokens.color.text)
                if (!facts.isNullOrEmpty()) KText(facts, tokens.type.micro, tokens.color.mute)
            }
            badge?.let { GitBadge(it) }
        }
        trailing?.invoke(this)
    }
}
