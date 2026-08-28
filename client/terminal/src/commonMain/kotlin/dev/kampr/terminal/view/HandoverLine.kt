package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.announce
import dev.kampr.terminal.file.Handover

// The one place a refused paste can be said. The node's error names this pane and is quiet
// everywhere else, so a strip that only ever showed the success would leave a refusal silent.
@Composable
fun HandoverLine(handover: Handover, to: String? = null, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val (words, tone) = when (handover) {
        Handover.Idle -> return
        is Handover.Going -> "sending ${handover.name}" to tokens.color.working
        is Handover.Sent ->
            "${handover.name} is on ${to ?: "the pane"}'s machine, and its path is typed in" to tokens.color.done
        is Handover.Refused -> handover.reason to tokens.color.blocked
    }
    KText(
        words,
        tokens.type.micro,
        tone,
        modifier
            .fillMaxWidth()
            .background(tokens.color.surface2)
            .padding(horizontal = 12.dp, vertical = 6.dp)
            .announce(words),
        maxLines = 3,
    )
}
