package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr

@Composable
fun Toggle(on: Boolean, title: String, detail: String, onChange: (Boolean) -> Unit) {
    val tokens = Kampr.tokens
    Row(
        Modifier
            .fillMaxWidth()
            .touchable()
            .action(
                "$title. $detail",
                { onChange(!on) },
                role = Role.Switch,
                selected = on,
                state = if (on) "on" else "off",
            ),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(11.dp),
    ) {
        Box(
            Modifier
                .width(40.dp)
                .height(23.dp)
                .background(
                    if (on) tokens.color.accent else tokens.color.raise,
                    RoundedCornerShape(tokens.radii.pill),
                )
                .padding(2.dp),
            contentAlignment = if (on) Alignment.CenterEnd else Alignment.CenterStart,
        ) {
            Box(
                Modifier
                    .size(19.dp)
                    .background(
                        if (on) tokens.color.onAccent else tokens.color.dim,
                        RoundedCornerShape(tokens.radii.pill),
                    ),
            )
        }
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            KText(title, tokens.type.cardTitle, tokens.color.text)
            KText(detail, tokens.type.captionSmall, tokens.color.mute)
        }
    }
}
