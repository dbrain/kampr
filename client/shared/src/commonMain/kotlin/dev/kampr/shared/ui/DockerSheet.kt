package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ago
import dev.kampr.shared.model.containerPhrase
import dev.kampr.shared.model.formatBytes
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ContainerInfo
import dev.kampr.shared.wire.ContainerMore
import dev.kampr.shared.wire.DockerAction
import dev.kampr.shared.wire.DockerDetail
import dev.kampr.shared.wire.DockerStats
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.flat

private const val TIMELINE = 20

// Tall enough for a page of logs, and fixed: the poll answers every three seconds and a tab can be
// any length, and a sheet that resized with each was one the operator chased with the pointer.
private val DOCKER_SHEET_HEIGHT = 680.dp
private val DOCKER_SHEET_WIDTH = 1040.dp
private val TABS_WIDTH = 520.dp

private enum class DockerTab(val title: String) { Overview("Info"), Logs("Logs"), Stats("Stats"), Env("Env"), Mounts("Mounts") }

private val ALIVE = setOf("running", "restarting", "paused")

// One container, from the node that runs it (docs/19). The state and the actions follow the herd
// entry, which the node pushes on every change; everything else is the last `docker.detail`. No
// action asks first: whoever can reach this can already run `docker` on the machine.
@Composable
fun DockerSheet(
    breakpoint: Breakpoint,
    node: NodeInfo,
    container: ContainerInfo,
    detail: DockerDetail?,
    refusal: String?,
    now: Double,
    onManage: (ManageOp) -> Unit,
    onDismiss: () -> Unit,
) {
    val tokens = Kampr.tokens
    var tab by remember(container.id) { mutableStateOf(DockerTab.Overview) }
    fun send(action: DockerAction) = onManage(ManageOp.Docker(action, node.id, container.id))

    BottomSheet(breakpoint, onDismiss, height = DOCKER_SHEET_HEIGHT, wideWidth = DOCKER_SHEET_WIDTH) {
        DockerHeader(node, container, now, onDismiss)
        Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                val running = container.state == "running"
                val alive = container.state in ALIVE
                if (running) Chip("Shell in", true, { send(DockerAction.Shell) })
                Chip("Restart", false, { send(DockerAction.Restart) })
                if (alive) Chip("Stop", false, { send(DockerAction.Stop) }) else Chip("Start", false, { send(DockerAction.Start) })
                if (alive) Chip("Kill", false, { send(DockerAction.Kill) }, label = "Kill", danger = true)
                if (!alive) Chip("Remove", false, { send(DockerAction.Remove) }, label = "Remove", danger = true)
                if (container.child != null) Chip("Release", false, { send(DockerAction.Release) }) else Chip("Adopt", false, { send(DockerAction.Adopt) })
            }
            container.adoptError?.let { KText(it, tokens.type.captionSmall, tokens.color.blocked, maxLines = 4) }
            refusal?.let { KText(it, tokens.type.captionSmall, tokens.color.blocked, Modifier.announce(it, urgent = true), maxLines = 3) }
            Segmented(DockerTab.entries.map { it.title }, tab.ordinal, { tab = DockerTab.entries[it] }, Modifier.widthIn(max = TABS_WIDTH).fillMaxWidth(), what = "tab")
        }
        Box(Modifier.weight(1f).fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 10.dp, bottom = 16.dp)) {
            val more = detail?.more
            if (more == null) {
                Spinner("Asking the node", modifier = Modifier.align(Alignment.Center))
            } else {
                when (tab) {
                    DockerTab.Overview -> Overview(container, detail, now)
                    DockerTab.Logs -> Pre(more.logs.map { it.line }, "no logs yet", follow = true)
                    DockerTab.Stats -> Stats(more)
                    DockerTab.Env -> Pre(more.env, "no environment")
                    DockerTab.Mounts -> Pre(more.mounts.map { m -> "${m.src} → ${m.dst}" + if (m.mode.isNotEmpty()) " (${m.mode})" else "" }, "no mounts")
                }
            }
        }
    }
}

@Composable
private fun DockerHeader(node: NodeInfo, container: ContainerInfo, now: Double, onDismiss: () -> Unit) {
    val tokens = Kampr.tokens
    Row(
        Modifier.fillMaxWidth().padding(start = 20.dp, top = 14.dp, end = 12.dp, bottom = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp), itemVerticalAlignment = Alignment.CenterVertically) {
                KText(container.name, tokens.type.paneTitle, tokens.color.text, Modifier.asHeading(), overflow = ELIDE_MIDDLE)
                for ((text, tone) in stateChips(container)) StateChip(text, tone)
            }
            LabelText(listOf(container.image, node.name, containerPhrase(container, now)).joinToString(" · "), tokens.type.micro, tokens.color.mute)
        }
        CloseAction("Close ${container.name}", onDismiss, target = LANDSCAPE_TOUCH)
    }
}

@Composable
private fun stateChips(container: ContainerInfo): List<Pair<String, Color>> {
    val color = Kampr.tokens.color
    val failed = container.state in setOf("exited", "dead") && (container.exitCode ?: 0) != 0
    return listOfNotNull(
        listOfNotNull(container.state.ifEmpty { "unknown" }, container.exitCode?.takeIf { failed }?.toString()).joinToString(" ") to when {
            container.state == "running" -> color.done
            container.state == "restarting" -> color.working
            failed -> color.blocked
            else -> color.idle
        },
        container.health?.let {
            it to when (it) {
                "healthy" -> color.done
                "unhealthy" -> color.blocked
                else -> color.working
            }
        },
        ("OOM" to color.blocked).takeIf { container.oom },
        ("↻${container.restarts}" to color.working).takeIf { container.restarts > 0 },
    )
}

@Composable
private fun Overview(container: ContainerInfo, detail: DockerDetail, now: Double) {
    val more = detail.more
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(14.dp)) {
        Facts(
            listOfNotNull(
                "status" to containerPhrase(container, now),
                flat(more.command)?.let { "command" to it },
                flat(more.restartPolicy)?.let { "restart" to it },
                (detail.entry.ports.ifEmpty { container.ports }).takeIf { it.isNotEmpty() }?.let { "ports" to it.joinToString("  ") },
                more.networks.takeIf { it.isNotEmpty() }?.let { "networks" to it.joinToString(", ") },
                container.project?.let { "compose" to listOfNotNull(it, container.service).joinToString(" / ") },
                container.lastCrash?.let { crash -> "last crash" to listOfNotNull("exit ${crash.exitCode}", "OOM".takeIf { crash.oom }, ago(crash.at, now)).joinToString(" · ") },
            ),
        )
        more.events.takeIf { it.isNotEmpty() }?.let { events ->
            Titled("Timeline") {
                Lines(
                    events.sortedByDescending { it.at }.take(TIMELINE).map { e ->
                        listOfNotNull(ago(e.at, now), listOfNotNull(e.action, e.exitCode?.toString(), e.signal, e.health).joinToString(" ")).joinToString(" · ")
                    },
                )
            }
        }
        more.healthLog.takeIf { it.isNotEmpty() }?.let { probes ->
            Titled("Health") {
                Lines(probes.reversed().map { p -> listOfNotNull(p.exitCode?.let { "exit $it" }, p.output.trim().ifEmpty { null }).joinToString(" · ") })
            }
        }
    }
}

@Composable
private fun Titled(label: String, content: @Composable ColumnScope.() -> Unit) {
    val tokens = Kampr.tokens
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        LabelText(label, tokens.type.micro, tokens.color.mute)
        content()
    }
}

@Composable
private fun Facts(rows: List<Pair<String, String>>) {
    val tokens = Kampr.tokens
    Column(verticalArrangement = Arrangement.spacedBy(5.dp)) {
        for ((key, value) in rows) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                LabelText(key, tokens.type.micro, tokens.color.mute, Modifier.width(FACT_KEY).padding(top = 2.dp))
                KText(value, tokens.type.meta, tokens.color.text, Modifier.weight(1f), maxLines = 3)
            }
        }
    }
}

private val FACT_KEY = 84.dp

@Composable
private fun Lines(lines: List<String>) {
    val tokens = Kampr.tokens
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        for (line in lines) KText(line, tokens.type.meta, tokens.color.text, maxLines = 4)
    }
}

// Machine output, as the machine wrote it: one line per line, never wrapped, scrolled both ways
// inside its own box so a long line costs a sideways scroll and not the shape of the page.
@Composable
private fun Pre(lines: List<String>, empty: String, follow: Boolean = false) {
    val tokens = Kampr.tokens
    if (lines.isEmpty()) {
        KText(empty, tokens.type.captionSmall, tokens.color.mute)
        return
    }
    val down = rememberScrollState()
    if (follow) LaunchedEffect(lines.size, down.maxValue) { down.scrollTo(down.maxValue) }
    val shape = RoundedCornerShape(tokens.radii.sm)
    Box(
        Modifier
            .fillMaxSize()
            .background(tokens.color.surface, shape)
            .edge(tokens.card, shape)
            .verticalScroll(down)
            .horizontalScroll(rememberScrollState())
            .padding(horizontal = 12.dp, vertical = 10.dp),
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            for (line in lines) KText(line.ifEmpty { " " }, tokens.type.meta, tokens.color.text, overflow = TextOverflow.Clip)
        }
    }
}

@Composable
private fun Stats(more: ContainerMore) {
    val tokens = Kampr.tokens
    val stats = more.stats
    if (stats == null) {
        KText("Usage is read only while the container runs.", tokens.type.captionSmall, tokens.color.mute)
        return
    }
    val memLimit = stats.memLimit?.takeIf { it > 0 } ?: more.limits?.memory?.takeIf { it > 0 }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).widthIn(max = 560.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        stats.cpuPct?.let { Meter("CPU", "${(it * 10).toLong() / 10.0}%", (it / 100).toFloat()) }
        stats.memUsed?.let { used ->
            Meter(
                "Memory",
                formatBytes(used) + (memLimit?.let { " of ${formatBytes(it)}" } ?: ", no limit"),
                memLimit?.let { (used.toDouble() / it).toFloat() },
            )
        }
        Facts(io(stats))
    }
}

private fun io(stats: DockerStats): List<Pair<String, String>> = listOfNotNull(
    stats.pids?.let { "processes" to it.toString() },
    if (stats.netRx != null || stats.netTx != null) "network" to "↓ ${formatBytes(stats.netRx ?: 0)}  ↑ ${formatBytes(stats.netTx ?: 0)}" else null,
    if (stats.blkRead != null || stats.blkWrite != null) "disk" to "read ${formatBytes(stats.blkRead ?: 0)}  written ${formatBytes(stats.blkWrite ?: 0)}" else null,
)

@Composable
private fun Meter(label: String, value: String, fraction: Float?) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Column(verticalArrangement = Arrangement.spacedBy(5.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            LabelText(label, tokens.type.micro, tokens.color.mute)
            KText(value, tokens.type.meta, tokens.color.text)
        }
        if (fraction != null) {
            val shown = fraction.coerceIn(0f, 1f)
            val tone = if (shown > 0.9f) {
                tokens.color.blocked
            } else if (shown > 0.7f) {
                tokens.color.working
            } else {
                tokens.color.done
            }
            Box(Modifier.fillMaxWidth().height(METER).background(tokens.color.raise, shape)) {
                Box(Modifier.fillMaxWidth(shown).fillMaxHeight().background(tone, shape))
            }
        }
    }
}

private val METER: Dp = 6.dp
