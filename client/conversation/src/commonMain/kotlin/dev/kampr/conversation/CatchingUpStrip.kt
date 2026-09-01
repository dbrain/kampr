package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.announce

// What a reader is looking at when it is not the conversation as it stands now.
//
// The transcript went out of date silently and the grid beside it did not, because only one of
// them is warm: a pane's stream is held across a re-watch by the registry (#252) while the
// conversation is opened by the pump the watch created, so reopening a pane means resolve, fold and
// page before anything is true again. Nothing pruned what was already drawn in that gap — a
// conversation from a session that had since been `/clear`ed sat there looking live (#393).
//
// Deliberately the same shape and language as `WorkingStrip`: a bar of colour and a word. It sits
// at the head of the transcript rather than inline with a turn, because it is about the whole of
// what is drawn and not about the last thing in it — and in the mute rather than the working
// colour, because nothing is happening. That is the difference a reader has to see at a glance.
fun catchingUp(status: ConnectionStatus, confirmed: Boolean, drawn: Boolean): String? = when {
    // Nothing drawn is not out of date, it is empty, and the transcript says so itself.
    !drawn -> null
    confirmed && status !is ConnectionStatus.Offline && status !is ConnectionStatus.Connecting -> null
    status is ConnectionStatus.Offline -> "offline — showing the last conversation this device saw"
    else -> "catching up — this is the conversation as it was"
}

@Composable
fun CatchingUpStrip(said: String, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    Row(
        modifier.fillMaxWidth().announce(said).padding(horizontal = 16.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        Box(
            Modifier.size(width = 7.dp, height = 13.dp)
                .background(tokens.color.mute, RoundedCornerShape(tokens.radii.sm)),
        )
        KText(said, tokens.type.meta, tokens.color.mute)
    }
}
