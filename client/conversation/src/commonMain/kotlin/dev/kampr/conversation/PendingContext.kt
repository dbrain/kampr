package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.md.Markdown
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.edge
import dev.kampr.shared.wire.ServerMsg

// A command a screen long scrolls in its own box rather than pushing the answers off a phone.
private val COMMAND_CEILING = 120.dp

// A plan is the whole of what is being approved and runs to thousands of characters, so it is
// given most of a phone; it still scrolls, because the answers under it are the point of the card.
private val PLAN_CEILING = 300.dp

private fun ServerMsg.Pending.isPlan(): Boolean = source == "hook" && header == "Plan"

@Composable
internal fun PendingContext(pending: ServerMsg.Pending, context: String) {
    val tokens = Kampr.tokens
    val box = RoundedCornerShape(tokens.radii.sm)
    val plan = pending.isPlan()
    Box(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.surface, box)
            .edge(tokens.card, box)
            .heightIn(max = if (plan) PLAN_CEILING else COMMAND_CEILING)
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 10.dp, vertical = 8.dp),
    ) {
        if (plan) {
            Markdown(context, query = "")
        } else {
            KText(
                context,
                tokens.type.caption.copy(fontFamily = tokens.fonts.mono),
                tokens.color.text,
                maxLines = Int.MAX_VALUE,
            )
        }
    }
}
