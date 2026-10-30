package dev.kampr.conversation.files

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.asHeading

@Composable
fun gitTone(letter: String): Color {
    val color = Kampr.tokens.color
    return when (letter) {
        "A" -> color.done
        "D", "U" -> color.blocked
        "R", "C" -> color.accent
        "?" -> color.mute
        else -> color.working
    }
}

@Composable
fun GitBadge(letter: String, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    Box(modifier.widthIn(min = 14.dp), contentAlignment = Alignment.Center) {
        KText(letter, tokens.type.micro.copy(fontFamily = tokens.fonts.mono), gitTone(letter))
    }
}

@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    LabelText(text, tokens.type.micro, tokens.color.mute, modifier.padding(top = 14.dp, bottom = 4.dp).asHeading())
}

@Composable
fun <T> LoadGate(load: Load<T>, what: String, onRetry: () -> Unit, content: @Composable (T) -> Unit) {
    val tokens = Kampr.tokens
    when (load) {
        is Load.Ready -> content(load.value)

        Load.Loading -> KText(
            "asking the node for $what",
            tokens.type.captionSmall,
            tokens.color.working,
            Modifier.padding(16.dp).announce("Asking the node for $what"),
        )

        is Load.Failed -> Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            KText(load.reason, tokens.type.captionSmall, tokens.color.blocked, Modifier.announce(load.reason), maxLines = 4)
            QuietAction("Try again", onRetry, Modifier.fillMaxWidth(), label = "Ask the node for $what again")
        }
    }
}
