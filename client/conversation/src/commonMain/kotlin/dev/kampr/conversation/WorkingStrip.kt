package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.wallClockMillis
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.announce
import dev.kampr.shared.util.elapsedSpan
import dev.kampr.shared.util.parseIsoMillis
import dev.kampr.shared.wire.Block
import kotlinx.coroutines.delay

// What the agent is doing, named by what it is actually doing. A harness that is running `Bash`
// says `Bash`; one that is writing an answer onto its own screen says so; anything else is
// working, which is the only word left that is true. Deliberately not a rotating vocabulary of
// invented gerunds — every word here is read off the transcript, and a reader who cannot tell an
// invented word from a measured one cannot trust either.
fun workingVerb(reply: Reply?): String {
    val turns = reply?.turns ?: return "working"
    // Whichever came *last*, rather than a tool call before a preview or the other way round. A
    // call is marked running until a record says otherwise and no record is written when a harness
    // is killed, so "running" is a claim that goes stale where a preview scraped off the screen
    // this second cannot. Position is the only thing that can tell them apart.
    for (turn in turns.asReversed()) {
        if (turn.id == LIVE_TURN_ID) return "writing"
        turn.blocks.filterIsInstance<Block.Tool>().lastOrNull { it.state == TOOL_RUNNING }
            ?.let { return it.name }
    }
    return "working"
}

// How long this reply has been going, from the stamp its own first turn carries. Nothing on the
// wire says when an agent *started* — but a reply's first record was written when it started, so
// the transcript answers it without a new field and without the client having to have been
// watching. A device that reconnects mid-answer gets the real figure rather than a stopwatch that
// begins when it happened to arrive.
fun workingSince(reply: Reply?, nowMillis: Double): String? {
    val at = parseIsoMillis(reply?.at) ?: return null
    return elapsedSpan(nowMillis - at)
}

// Ticked by the second, and it is the one thing on this screen that has to be: a counter that
// moves once a minute reads as a frozen one, which is the whole complaint that the age beside a
// turn used to earn (#285). It ticks only while it is on the screen, which is only while an agent
// is working.
@Composable
fun WorkingStrip(reply: Reply?, modifier: Modifier = Modifier, clock: () -> Double = ::wallClockMillis) {
    val tokens = Kampr.tokens
    var now by remember { mutableStateOf(clock()) }
    LaunchedEffect(reply?.key) {
        while (true) {
            delay(1000)
            now = clock()
        }
    }
    val verb = workingVerb(reply)
    val since = workingSince(reply, now)
    val said = if (since == null) "$verb…" else "$verb… ($since)"
    Row(
        modifier.fillMaxWidth().announce(if (since == null) "$verb" else "$verb, $since so far"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        Box(
            Modifier.size(width = 7.dp, height = 13.dp)
                .background(tokens.color.working, RoundedCornerShape(tokens.radii.sm)),
        )
        KText(said, tokens.type.meta, tokens.color.working)
    }
}
