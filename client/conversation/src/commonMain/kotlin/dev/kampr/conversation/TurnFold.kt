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
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.util.relativeTime
import dev.kampr.shared.wire.Turn

// Folded by key, in the same set the tool runs and the tool cards toggle through, so what the
// reader put away stays away across every tick of the transcript under it.
fun foldKey(turn: Turn): String? = if (foldable(turn)) "fold:${turn.id}" else null

// A header is a row of chrome carrying a 36 dp target, so it has to buy more than it costs, which
// is the same test the tool runs get. A reply is short by construction and already says who wrote
// it by where it sits; a turn of nothing but calls is the run's business and wears a chevron
// already; a preview is being rewritten under the reader and folding what is changing is the one
// thing a fold must not do. What is left earns a header once it runs past a couple of lines or
// holds more than one thing.
fun foldable(turn: Turn): Boolean {
    if (turn.role == "user" || turn.id == LIVE_TURN_ID) return false
    val pieces = groupBlocks(turn.blocks)
    if (pieces.isEmpty() || pieces.all { it is Piece.Call }) return false
    return pieces.size > 1 || turnText(turn).count { it == '\n' } >= 2
}

// An age, not a time of day. `at` is whatever the harness wrote and the node copied, and nothing
// on the wire says which offset it is in — `parseIsoMillis` reads the fields and ignores the zone
// — so a clock face here would be wrong by up to half a day and would look right while it was.
fun turnStamp(at: String?, nowMillis: Double): String? = relativeTime(at, nowMillis).takeIf { it != "—" }

private val DECORATION = Regex("^(#{1,6}|>+|[-*+]|\\d+\\.)\\s+")

// One line of prose for a folded turn to show of itself. Markdown's own punctuation is stripped
// rather than rendered: a row reading "## Corrections and event" is the syntax, not the message.
fun turnGist(turn: Turn): String =
    turnText(turn)
        .lineSequence()
        .map { it.trim() }
        .firstOrNull { it.isNotEmpty() }
        ?.replace(DECORATION, "")
        ?.replace("**", "")
        ?.replace("`", "")
        ?.trim()
        .orEmpty()

@Composable
fun TurnHeader(
    stamp: String?,
    gist: String,
    parts: Int,
    folded: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val held = listOfNotNull(
        stamp,
        gist.takeIf { it.isNotEmpty() },
        if (parts > 1) "$parts parts" else null,
    ).joinToString(", ")
    DisableSelection {
        Row(
            modifier
                .fillMaxWidth()
                .touchable(LANDSCAPE_TOUCH)
                .action(
                    if (folded) "Show the message of $held" else "Hide the message of $held",
                    onToggle,
                    selected = !folded,
                ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (stamp != null) KText(stamp, tokens.type.micro, tokens.color.mute)
            if (folded) KText(gist, tokens.type.meta, tokens.color.dim, Modifier.weight(1f))
            else Box(Modifier.weight(1f))
            IconGlyph(
                if (folded) ConversationIcons.chevronDown else ConversationIcons.chevronUp,
                12.dp,
                tokens.color.mute,
            )
        }
    }
}
