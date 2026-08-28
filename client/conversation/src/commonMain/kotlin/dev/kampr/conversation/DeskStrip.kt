package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.DeskLine
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.touchable

// What sending would actually do, said in the words of the thing it would do it to.
//
// Herdr's `pane.send_text` **appends**, so a reply typed here does not replace what is on the
// pane's line — it is added to the end of it, and the two submit together as one sentence. That is
// occasionally what somebody wants and it was never visible, which is the whole complaint.
fun deskWords(line: DeskLine, agent: String?): String =
    "Waiting in ${agent ?: "the agent"}'s box — your reply is added to the end of it"

// Whether the operator can take the line off the pane rather than adding to it.
//
// Two conditions and both are real: a read-only device may not write to a pane at all, and a
// harness the node has not measured a clearing keystroke for carries none. **A guessed key is
// worse than no button**: `ctrl+u` takes a single visual row of Claude's wrapped buffer and leaves
// the rest, and `ctrl+c` arms an exit on agy rather than clearing anything.
fun deskTakeable(line: DeskLine?, enabled: Boolean): Boolean = enabled && line?.clear != null

// The pane's own half-sentence, above the box that would be added to it.
//
// A read-only device sees this: it is the pane's state, not this client's, and somebody who cannot
// type still wants to know what is sitting there. They simply get no button.
@Composable
fun DeskStrip(
    line: DeskLine?,
    agent: String?,
    enabled: Boolean,
    onTakeOver: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    if (line == null) return
    val shape = RoundedCornerShape(tokens.radii.md)
    Column(
        modifier
            .fillMaxWidth()
            .padding(start = 12.dp, end = 12.dp, top = 9.dp)
            .background(tokens.color.raise, shape)
            .edge(BorderSpec(1.dp, tokens.color.working), shape)
            .announce("${deskWords(line, agent)}. It reads: ${line.text}")
            .padding(horizontal = 11.dp, vertical = 9.dp),
        verticalArrangement = Arrangement.spacedBy(5.dp),
    ) {
        Row(
            Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.Top,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            KText(line.text, tokens.type.body, tokens.color.text, Modifier.weight(1f), maxLines = 3)
            if (deskTakeable(line, enabled)) {
                val chip = RoundedCornerShape(tokens.radii.sm)
                Box(
                    Modifier
                        .background(tokens.color.surface, chip)
                        .edge(tokens.card, chip)
                        .touchable()
                        .action("Take that line off the pane and put it in this reply box", onTakeOver, chip)
                        .padding(horizontal = 10.dp, vertical = 6.dp),
                ) {
                    KText("Take it over", tokens.type.buttonSmall, tokens.color.text)
                }
            }
        }
        KText(deskWords(line, agent), tokens.type.micro, tokens.color.mute, maxLines = 2)
    }
}
