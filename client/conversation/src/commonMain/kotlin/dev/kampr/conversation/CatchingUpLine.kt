package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.announce

// Where this device's reading of the transcript stops.
//
// The transcript went out of date silently and the grid beside it did not, because only one of
// them is warm: a pane's stream is held across a re-watch by the registry (#252) while the
// conversation is opened by the pump the watch created, so reopening a pane means resolve, fold and
// page before anything is true again. Nothing pruned what was already drawn in that gap — a
// conversation from a session that had since been `/clear`ed sat there looking live (#393).
//
// **It is a boundary and not a wash.** This began as an alpha over the whole list, which is the
// claim that everything drawn is doubtful — and it is not: those turns were read off the
// transcript and they are exactly right. What is missing is whatever comes *after* them, so the
// thing to draw is the edge, at the foot of the list where the end of the conversation already is
// and where a transcript pinned to its own end puts the reader's eye. Greying the words that are
// true to warn about the words that are absent costs legibility to say something false.
fun catchingUp(status: ConnectionStatus, confirmed: Boolean, drawn: Boolean): String? = when {
    // Nothing drawn is not out of date, it is empty, and the transcript says so itself.
    !drawn -> null
    confirmed && status !is ConnectionStatus.Offline && status !is ConnectionStatus.Connecting -> null
    status is ConnectionStatus.Offline -> "read up to here — offline"
    else -> "read up to here"
}

@Composable
fun CatchingUpLine(said: String, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    Row(
        modifier.fillMaxWidth().announce(said).padding(top = 10.dp, bottom = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        KText(said, tokens.type.meta, tokens.color.mute)
        Box(Modifier.weight(1f).height(1.dp).background(tokens.color.line))
    }
}
