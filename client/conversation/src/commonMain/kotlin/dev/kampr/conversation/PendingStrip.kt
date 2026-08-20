package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.Mark
import dev.kampr.shared.ui.MarkShape
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.wire.ServerMsg

// `pending.source` records whether the node lifted the question from the transcript or from the
// screen. It is a provenance note, never a branch: the strip and the answer are identical either
// way, and the node decides whether a submit key follows the one it is sent.
@Composable
fun PendingStrip(pending: ServerMsg.Pending, onAnswer: (String) -> Unit, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val question = pending.question ?: return
    val shape = RoundedCornerShape(tokens.radii.md)
    Column(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 9.dp)
            .background(tokens.color.blockedBg, shape)
            .edge(BorderSpec(1.dp, tokens.color.blocked), shape)
            .announce(
                "The agent is asking: $question. ${pending.options.size} answers: " +
                    pending.options.joinToString(", ") { "${it.key} ${it.label}" },
                urgent = true,
            )
            .padding(horizontal = 13.dp, vertical = 11.dp),
        verticalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Mark(tokens.color.blocked, MarkShape.Square, 7.dp)
            KText(question, tokens.type.bodyStrong, tokens.color.text, Modifier.weight(1f), maxLines = 3)
        }
        Row(
            Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(7.dp),
        ) {
            pending.options.forEachIndexed { index, option ->
                val primary = index == 0
                val chip = RoundedCornerShape(tokens.radii.sm)
                Box(
                    Modifier
                        .widthIn(min = 96.dp)
                        .background(if (primary) tokens.color.accent else tokens.color.raise, chip)
                        .edge(if (primary) BorderSpec(0.dp, tokens.color.accent) else tokens.card, chip)
                        .touchable()
                        .action("Answer ${option.key}, ${option.label}", { onAnswer(option.key) }, chip)
                        .padding(horizontal = 12.dp, vertical = 9.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    KText(
                        "${option.key} · ${option.label}",
                        tokens.type.buttonSmall,
                        if (primary) tokens.color.onAccent else tokens.color.text,
                    )
                }
            }
        }
        KText(
            when (pending.source) {
                "transcript" -> "Read from the transcript, answered by keys into the pane."
                else -> "Read from the pane, answered by keys into the pane."
            },
            tokens.type.micro,
            tokens.color.mute,
            maxLines = 2,
        )
    }
}
