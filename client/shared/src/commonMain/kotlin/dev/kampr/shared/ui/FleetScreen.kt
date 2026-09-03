package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Cohort
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.Matching
import dev.kampr.shared.model.cohorts
import dev.kampr.shared.model.matching
import dev.kampr.shared.model.fleetTargets
import dev.kampr.shared.model.recipients
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.Palette
import dev.kampr.shared.wire.FleetInfo
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Question

// The fleet board: every run, grouped by the fan-out that produced it, with what needs somebody at
// the top.
//
// It answers two questions and is laid out in that order — *which host needs me*, then *how did
// they all go*. A waiting host shows its question inline with the choices the prompt declared for
// itself, so the commonest reply is one tap without opening anything.
@Composable
fun FleetScreen(
    herd: Herd,
    breakpoint: Breakpoint,
    onOpenPane: (String) -> Unit,
    onAnswer: (paneId: String, text: String) -> Unit,
    onStop: (paneId: String) -> Unit,
    onRun: (command: String) -> Unit,
    canRun: Boolean,
    // The node's memory of what has been run here, and the two ops that curate it. It arrives
    // unasked on the greeting and again after every change, so this screen never asks for it.
    book: ServerMsg.FleetBook = ServerMsg.FleetBook(),
    onBook: (ManageOp) -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val cohorts = herd.cohorts()
    val targets = fleetTargets(herd.nodes)
    var confirming by remember { mutableStateOf<PendingBroadcast?>(null) }
    var composing by remember { mutableStateOf(false) }

    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        Row(
            Modifier.fillMaxWidth().padding(start = 20.dp, top = 16.dp, end = 20.dp, bottom = 11.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            KText("Fleet", tokens.type.screenTitle, tokens.color.text, Modifier.asHeading())
            if (canRun && targets.isNotEmpty()) {
                FleetChip("Run", isDefault = true) { composing = true }
            }
        }
        if (cohorts.isEmpty()) {
            EmptyBoard(targets.size, canRun)
        } else {
            LazyColumn(
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(16.dp),
                verticalArrangement = Arrangement.spacedBy(20.dp),
            ) {
                items(cohorts, key = { it.id }) { cohort ->
                    CohortBlock(cohort, herd, onOpenPane, onAnswer, onStop) { confirming = it }
                }
            }
        }
    }

    if (composing) {
        RunSheet(
            hosts = targets.size,
            book = book,
            breakpoint = breakpoint,
            onCancel = { composing = false },
            onRun = {
                onRun(it)
                composing = false
            },
            onBook = onBook,
        )
    }

    confirming?.let { pending ->
        BroadcastConfirm(
            pending = pending,
            breakpoint = breakpoint,
            onCancel = { confirming = null },
            onConfirm = {
                pending.recipients.forEach { onAnswer(it, pending.answer) }
                confirming = null
            },
        )
    }
}

// One answer, about to reach more than one machine. Held until the operator has seen exactly which.
private data class PendingBroadcast(
    val answer: String,
    val label: String,
    val prompt: String,
    val recipients: List<String>,
    val hostNames: List<String>,
    val differingNames: List<String>,
)

@Composable
private fun EmptyBoard(hosts: Int, canRun: Boolean) {
    val tokens = Kampr.tokens
    Column(
        modifier = Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        KText("No fleet runs", tokens.type.paneTitle, tokens.color.text)
        KText(
            when {
                !canRun -> "This device can watch fleet runs but not start them."
                hosts == 0 -> "No machine in this herd can be reached right now."
                else -> "One command, on all $hosts machines you can reach."
            },
            tokens.type.caption,
            tokens.color.mute,
            maxLines = 2,
        )
    }
}

@Composable
private fun CohortBlock(
    cohort: Cohort,
    herd: Herd,
    onOpenPane: (String) -> Unit,
    onAnswer: (String, String) -> Unit,
    onStop: (String) -> Unit,
    onBroadcast: (PendingBroadcast) -> Unit,
) {
    val tokens = Kampr.tokens
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        KText(cohort.command, tokens.type.cardTitle, tokens.color.accent)
        Tally(cohort)
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .background(tokens.color.surface2, RoundedCornerShape(tokens.radii.md)),
        ) {
            cohort.panes.forEach { pane ->
                HostRow(pane, herd, onOpenPane, onAnswer, onStop, onBroadcast)
            }
        }
    }
}

@Composable
private fun Tally(cohort: Cohort) {
    val tokens = Kampr.tokens
    // Order matters: what needs somebody reads first, and a success is the last thing anybody
    // needs to be told about.
    val parts = listOf(
        cohort.waiting to ("need you" to tokens.color.blocked),
        cohort.running to ("running" to tokens.color.working),
        cohort.quiet to ("quiet" to tokens.color.idle),
        cohort.failed to ("failed" to tokens.color.blocked),
        cohort.succeeded to ("done" to tokens.color.done),
    ).filter { it.first > 0 }
    Row(horizontalArrangement = Arrangement.spacedBy(14.dp)) {
        parts.forEach { (count, said) ->
            KText("$count ${said.first}", tokens.type.caption, said.second)
        }
    }
}

@Composable
private fun HostRow(
    pane: PaneInfo,
    herd: Herd,
    onOpenPane: (String) -> Unit,
    onAnswer: (String, String) -> Unit,
    onStop: (String) -> Unit,
    onBroadcast: (PendingBroadcast) -> Unit,
) {
    val tokens = Kampr.tokens
    val fleet = pane.fleet ?: return
    val host = herd.nodes.firstOrNull { it.id == pane.nodeId }?.name ?: pane.nodeId
    val said = describe(fleet, tokens.color)

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(if (fleet.isWaiting) tokens.color.blockedBg else tokens.color.surface2)
            .action("$host, ${said.first}. Open this run", { onOpenPane(pane.id) })
            .padding(horizontal = 14.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.width(120.dp)) {
                KText(host, tokens.type.bodyStrong, tokens.color.text)
            }
            KText(said.first, tokens.type.caption, said.second)
        }
        // The node cannot read this run at all, and a board that stayed silent would let a host
        // that is waiting look merely idle (probe #332).
        if (fleet.blind) {
            KText(
                "state unreadable — this command changes user",
                tokens.type.captionSmall,
                tokens.color.mute,
                maxLines = 2,
            )
        }
        fleet.question?.let { question ->
            QuestionBlock(pane, herd, question, onAnswer, onOpenPane, onBroadcast)
        }
        if (!fleet.isFinished) {
            KText(
                "Stop",
                tokens.type.captionSmall,
                tokens.color.mute,
                Modifier.action("Stop this run on $host", { onStop(pane.id) }),
            )
        }
    }
}

@Composable
private fun QuestionBlock(
    pane: PaneInfo,
    herd: Herd,
    question: Question,
    onAnswer: (String, String) -> Unit,
    onOpenPane: (String) -> Unit,
    onBroadcast: (PendingBroadcast) -> Unit,
) {
    val tokens = Kampr.tokens
    val said = when {
        question.isSecret -> "Asking for a password"
        question.ownsTheScreen -> "This one has taken the whole screen"
        question.prompt.isBlank() -> "Waiting, having said nothing"
        else -> question.prompt
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        KText(said, tokens.type.body, tokens.color.text, maxLines = 3)
        // Weaker evidence, and it says so rather than passing for a measurement (probe #341).
        if (question.inferred) {
            KText("looks like it is asking", tokens.type.captionSmall, tokens.color.mute)
        }
        // Every rung that is not two buttons ends in the same place: open the pane and type. The
        // fallback is always available, which is why no pattern here is load-bearing.
        if (question.answerable.isEmpty()) {
            KText(
                "Open to answer",
                tokens.type.caption,
                tokens.color.accent,
                Modifier.action("Open this run to answer it", { onOpenPane(pane.id) }),
            )
            return@Column
        }
        val match = herd.matching(pane.id).getOrNull()
        val broadcasts = match != null && match.reach > 1
        // A fleet answer is `input`, which is `typing`, which is dropped over a socket that is not
        // live — the pane's own chips and these are the same defect. `undelivered` is nothing here
        // because a board pane is not watched, so nothing counts a lost press against it: what
        // bites on this screen is the socket, and that is what is read.
        val reach = answering(LocalConnectionStatus.current, 0)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            question.answerable.forEach { option ->
                FleetChip(option.label, option.key == question.defaultKey, enabled = reach.enabled) {
                    if (broadcasts) {
                        onBroadcast(pendingFor(match, herd, option.key, option.label, question.prompt))
                    } else {
                        onAnswer(pane.id, option.key)
                    }
                }
            }
        }
        reach.note?.let { KText(it, tokens.type.captionSmall, tokens.color.blocked, maxLines = 2) }
        if (broadcasts) {
            val n = match.others.size
            KText(
                "$n other host${if (n == 1) "" else "s"} asking the same thing",
                tokens.type.captionSmall,
                tokens.color.mute,
            )
        }
    }
}

private fun pendingFor(
    match: Matching,
    herd: Herd,
    key: String,
    label: String,
    prompt: String,
): PendingBroadcast {
    fun name(id: String): String {
        val nodeId = herd.panes.firstOrNull { it.id == id }?.nodeId ?: return id
        return herd.nodes.firstOrNull { it.id == nodeId }?.name ?: nodeId
    }
    return PendingBroadcast(
        answer = key,
        label = label,
        prompt = prompt,
        recipients = match.recipients(),
        hostNames = match.recipients().map(::name),
        differingNames = match.differing.map { name(it.id) },
    )
}

// One tap, several root shells. The hosts it will reach are named, and so are the ones it will
// not — the silent third of a fleet is what bites you.
@Composable
private fun BroadcastConfirm(
    pending: PendingBroadcast,
    breakpoint: Breakpoint,
    onCancel: () -> Unit,
    onConfirm: () -> Unit,
) {
    val tokens = Kampr.tokens
    BottomSheet(breakpoint, onCancel) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            KText("Answer ${pending.recipients.size} hosts", tokens.type.paneTitle, tokens.color.text)
            KText(pending.prompt, tokens.type.body, tokens.color.mute, maxLines = 3)
            KText(
                "Sending \"${pending.label}\" to ${pending.hostNames.joinToString(", ")}.",
                tokens.type.body,
                tokens.color.text,
                maxLines = 4,
            )
            if (pending.differingNames.isNotEmpty()) {
                KText(
                    "Not sending to ${pending.differingNames.joinToString(", ")} — asking something else.",
                    tokens.type.caption,
                    tokens.color.blocked,
                    maxLines = 3,
                )
            }
            val reach = answering(LocalConnectionStatus.current, 0)
            reach.note?.let { KText(it, tokens.type.caption, tokens.color.blocked, maxLines = 2) }
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                FleetChip("Cancel", isDefault = false, onClick = onCancel)
                FleetChip("Send to all", isDefault = true, enabled = reach.enabled, onClick = onConfirm)
            }
        }
    }
}

private fun describe(fleet: FleetInfo, color: Palette): Pair<String, androidx.compose.ui.graphics.Color> = when {
    fleet.state == "waiting" -> "needs you" to color.blocked
    fleet.state == "running" -> "running" to color.working
    fleet.state == "quiet" -> (fleet.quietSeconds?.let { "quiet ${it}s" } ?: "quiet") to color.idle
    fleet.succeeded -> "done" to color.done
    // A run the kernel killed has no exit code, and showing one would call a death a clean finish.
    fleet.signal != null -> "killed · signal ${fleet.signal}" to color.blocked
    fleet.exitCode != null -> "failed · exit ${fleet.exitCode}" to color.blocked
    fleet.isFinished -> "ended · no status" to color.blocked
    else -> fleet.state to color.idle
}
