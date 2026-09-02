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
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.Mark
import dev.kampr.shared.ui.MarkShape
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.touchable
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

// The fold lives in the transcript's own toggle set, keyed like a turn's, so this is the mechanism
// the tool cards already use rather than a second one beside it. That set holds departures from the
// default and this strip's default is shut, so membership means open — and because the set is not
// keyed on the list, a launch finishing or starting never folds the strip back under a reader who
// opened it.
const val RUNNING_OPEN = "running:open"

fun runningSince(run: Running, nowMillis: Double): String? =
    parseIsoMillis(run.since)?.let { elapsedSpan(nowMillis - it) }

// What a reader with no eyes on the strip is told, and it is never less than the eye is shown. A
// folded strip draws a count and the names are behind it, so the line names them anyway — losing
// them is the one thing a fold must not do to a screen reader. The stopwatches go with the rows
// they belong to: they move every second, this is a live region, and a folded strip re-read once a
// second is a fold that made the screen louder rather than quieter.
fun runningSpoken(running: List<Running>, rows: Boolean, nowMillis: Double): String {
    val head = "${running.size} still running"
    return running.joinToString("; ", prefix = if (rows) "$head: " else "$head, folded: ") { run ->
        val since = if (rows) runningSince(run, nowMillis) else null
        if (since == null) runningLabel(run) else "${runningLabel(run)}, $since"
    }
}

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
// Each row is a stopwatch rather than an age. `elapsedSpan` moves every second, and it has to: a
// counter that changes once a minute reads as a frozen one, and a frozen counter is exactly what
// somebody looking for a stuck shell would misread. Not a probe — an operator's reading of their
// own screen, twice: the complaint the age beside a turn used to earn, and then this strip's own,
// *"one is measuring in minutes so can't tell if it's ticking"*, against the `≥1h` branch that
// still rendered `2h 14m` while claiming to be a stopwatch.
//
// It starts folded, on the operator's own reading of it: *"id say it should collapse by default
// and just show numbers, or if one show the one running full line"*. Eight launches is eight rows
// standing over the transcript, and the count is the news — that something is outstanding — where
// the names are what somebody who has read the count then goes looking for.
//
// A single launch is not a special case of the fold, it is a size at which there is nothing to
// fold: the row names the thing and carries its stopwatch, which is strictly more than the count
// says, and it hides nothing. So there is no chevron there offering to hide it. The count line
// stands above the rows in every state, so a second launch starting changes what is under the
// header rather than the shape of the whole strip.
//
// It is a **read**. Nothing here presses anything, and nothing here reshapes or focuses a pane
// (rule 3) — the way in to a launched conversation is still the card the turn carries. The fold is
// this client's own state and goes nowhere near the pane.
@Composable
fun RunningStrip(
    running: List<Running>,
    modifier: Modifier = Modifier,
    open: Boolean = false,
    onFold: () -> Unit = {},
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
    val foldable = running.size > 1
    val rows = if (foldable && !open) emptyList() else running
    DisableSelection {
        Column(
            modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 6.dp)
                .background(tokens.color.raise, shape)
                .edge(tokens.card, shape)
                .announce(runningSpoken(running, rows.isNotEmpty(), now))
                .padding(horizontal = 12.dp, vertical = 9.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .then(
                        if (!foldable) {
                            Modifier
                        } else {
                            Modifier
                                .touchable(LANDSCAPE_TOUCH)
                                .action(
                                    if (open) "Hide what is running" else "Show what is running",
                                    onFold,
                                    selected = open,
                                )
                        },
                    ),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                LabelText(
                    if (running.size == 1) "1 still running" else "${running.size} still running",
                    tokens.type.metaSmall,
                    tokens.color.mute,
                    Modifier.weight(1f),
                )
                if (foldable) {
                    IconGlyph(
                        if (open) ConversationIcons.chevronUp else ConversationIcons.chevronDown,
                        12.dp,
                        tokens.color.mute,
                    )
                }
            }
            rows.forEach { run ->
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
