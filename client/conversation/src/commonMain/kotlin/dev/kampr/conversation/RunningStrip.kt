package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.DisableSelection
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
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.Mark
import dev.kampr.shared.ui.MarkShape
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.util.elapsedSpan
import dev.kampr.shared.util.parseIsoMillis
import dev.kampr.shared.wire.Running
import kotlinx.coroutines.delay

// What one launch is called, in the words the harness used. `kind` is an open string, so an
// unknown one is printed rather than mapped to a default — the node only ever sends a word it
// measured, and a harness that grows a third kind must read as itself here without a release.
fun runningLabel(run: Running): String {
    val what = run.title?.trim()?.takeIf { it.isNotEmpty() }
    val who = when (run.kind) {
        "agent" -> "agent"
        "shell" -> "shell"
        "" -> run.name?.trim()?.takeIf { it.isNotEmpty() } ?: "task"
        else -> run.kind
    }
    return what?.let { "$who · $it" } ?: who
}

fun runningSince(run: Running, nowMillis: Double): String? =
    parseIsoMillis(run.since)?.let { elapsedSpan(nowMillis - it) }

// The operator, on 0.1.49: *"sometimes claude leaves shells open forever and 'working' can mean
// nothing but 'a shell was left running'"*.
//
// **`working` is one word for two situations and the operator has to be able to tell them apart.**
// A pane reports `working` while anything at all is outstanding, and the transcript's own answer —
// a card in the turn that launched it — is only findable by scrolling back to the moment of the
// launch, which is the wrong place by definition: what is running now is a fact about now. So this
// is a fixed place, above the reply box, that holds while anything is in flight and disappears
// when nothing is.
//
// Each row is a stopwatch rather than an age. `elapsedSpan` moves every second, and it has to:
// a counter that changes once a minute reads as a frozen one (#285), and a frozen counter is
// exactly what somebody looking for a stuck shell would misread.
//
// It is a **read**. Nothing here presses anything, and nothing here reshapes or focuses a pane
// (rule 3) — the way in to a launched conversation is still the card the turn carries.
@Composable
fun RunningStrip(
    running: List<Running>,
    modifier: Modifier = Modifier,
    clock: () -> Double = ::wallClockMillis,
) {
    if (running.isEmpty()) return
    val tokens = Kampr.tokens
    var now by remember { mutableStateOf(clock()) }
    // Keyed on the set rather than on nothing, so the tick restarts with the list and a strip that
    // has just appeared does not wait out the remainder of somebody else's second.
    LaunchedEffect(running.map { it.call }) {
        now = clock()
        while (true) {
            delay(1000)
            now = clock()
        }
    }
    val shape = RoundedCornerShape(tokens.radii.md)
    DisableSelection {
        Column(
            modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 6.dp)
                .background(tokens.color.raise, shape)
                .edge(tokens.card, shape)
                .announce(
                    running.joinToString("; ", prefix = "${running.size} still running: ") { run ->
                        val since = runningSince(run, now)
                        if (since == null) runningLabel(run) else "${runningLabel(run)}, $since"
                    },
                )
                .padding(horizontal = 12.dp, vertical = 9.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            LabelText(
                if (running.size == 1) "1 still running" else "${running.size} still running",
                tokens.type.metaSmall,
                tokens.color.mute,
            )
            running.forEach { run ->
                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Mark(tokens.color.working, MarkShape.Circle, 7.dp)
                    IconGlyph(
                        if (run.kind == "agent") ConversationIcons.branch else KamprIcons.tool,
                        12.dp,
                        tokens.color.dim,
                    )
                    KText(
                        runningLabel(run),
                        tokens.type.meta,
                        tokens.color.text,
                        Modifier.weight(1f),
                        maxLines = 1,
                    )
                    runningSince(run, now)?.let {
                        KText(it, tokens.type.micro, tokens.color.working)
                    }
                }
            }
        }
    }
}
