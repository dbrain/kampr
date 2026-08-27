package dev.kampr.conversation

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.touchable

// What a reply says about itself in one line: how much of it there is, and — put away — the first
// thing it said. "Steps" rather than "turns", because a turn is the transcript's word for a record
// and a step is what the reader watched happen.
fun replyTally(reply: Reply): String {
    val steps = if (reply.steps == 1) "1 step" else "${reply.steps} steps"
    val calls = if (reply.tools == 1) "1 call" else "${reply.tools} calls"
    return if (reply.tools == 0) steps else "$steps · $calls"
}

fun replyLabel(agent: String?, stamp: String?, reply: Reply, collapsed: Boolean): String = listOfNotNull(
    agent ?: "agent",
    stamp,
    replyTally(reply),
    replyGist(reply).takeIf { collapsed && it.isNotEmpty() },
).joinToString(", ")

// The head of one reply, and the control that puts the whole thing away. This is the row a reader
// reaches for: what they want gone is an answer and its thirty tool calls, not one of the thirty,
// and until this existed the only way to clear a long reply off the screen was to scroll past it.
@Composable
fun ReplyHead(
    reply: Reply,
    agent: String?,
    now: Double,
    collapsed: Boolean,
    onToggle: () -> Unit,
    edge: BlockEdge,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val skin = speakerSkin(Speaker.Agent, agent)
    val stamp = replySpan(reply.at, reply.until, now)
    val held = replyLabel(agent, stamp, reply, collapsed)
    BlockFrame(skin, edge, modifier) {
        DisableSelection {
            Row(
                Modifier
                    .fillMaxWidth()
                    .touchable(LANDSCAPE_TOUCH)
                    .action(
                        if (collapsed) "Show the reply of $held" else "Put away the reply of $held",
                        onToggle,
                        selected = !collapsed,
                    ),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                LabelText(skin.label, tokens.type.metaSmall, skin.rail)
                if (stamp != null) KText(stamp, tokens.type.micro, tokens.color.mute)
                if (collapsed) {
                    KText(replyGist(reply), tokens.type.meta, tokens.color.dim, Modifier.weight(1f))
                } else {
                    KText(replyTally(reply), tokens.type.meta, tokens.color.mute, Modifier.weight(1f))
                }
                IconGlyph(
                    if (collapsed) ConversationIcons.chevronDown else ConversationIcons.chevronUp,
                    12.dp,
                    tokens.color.mute,
                )
            }
        }
    }
}
