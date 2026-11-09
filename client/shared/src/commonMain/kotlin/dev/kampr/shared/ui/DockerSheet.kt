package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ago
import dev.kampr.shared.model.containerPhrase
import dev.kampr.shared.model.formatBytes
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ContainerInfo
import dev.kampr.shared.wire.DockerAction
import dev.kampr.shared.wire.DockerDetail
import dev.kampr.shared.wire.DockerStats
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.flat

private const val LOG_LINES = 40
private const val TIMELINE = 20

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
    val compact = breakpoint != Breakpoint.Portrait
    val more = detail?.more
    fun send(action: DockerAction) = onManage(ManageOp.Docker(action, node.id, container.id))

    BottomSheet(breakpoint, onDismiss) {
        SheetHeader(container.name, "${container.image} · ${containerPhrase(container, now)} · ${node.name}", null, onDismiss, compact)
        Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                val running = container.state == "running"
                if (running) Chip("Shell in", false, { send(DockerAction.Shell) })
                Chip("Restart", false, { send(DockerAction.Restart) })
                if (running || container.state == "restarting" || container.state == "paused") {
                    Chip("Stop", false, { send(DockerAction.Stop) })
                } else {
                    Chip("Start", false, { send(DockerAction.Start) })
                }
                if (container.child != null) {
                    Chip("Release", false, { send(DockerAction.Release) }, quiet = true)
                } else {
                    Chip("Adopt", false, { send(DockerAction.Adopt) }, quiet = true)
                }
            }
            container.adoptError?.let {
                KText(it, tokens.type.captionSmall, tokens.color.blocked, Modifier.padding(top = 8.dp), maxLines = 4)
            }
            refusal?.let {
                KText(it, tokens.type.captionSmall, tokens.color.blocked, Modifier.padding(top = 8.dp).announce(it, urgent = true), maxLines = 3)
            }
            Facts(
                listOfNotNull(
                    listOfNotNull(container.state, container.health, container.restarts.takeIf { it > 0 }?.let { "↻$it" }).joinToString(" · "),
                    flat(more?.command)?.let { "command $it" },
                    flat(more?.restartPolicy)?.let { "restart $it" },
                    (detail?.entry?.ports.orEmpty().ifEmpty { container.ports }).takeIf { it.isNotEmpty() }?.joinToString("  "),
                    more?.networks?.takeIf { it.isNotEmpty() }?.joinToString(", ")?.let { "networks $it" },
                ),
            )
            if (detail == null) KText("Asking the node…", tokens.type.captionSmall, tokens.color.mute, Modifier.padding(top = 10.dp))
            more?.stats?.let { Usage(it, more.limits?.memory) }
            more?.events?.takeIf { it.isNotEmpty() }?.let { events ->
                Section("Timeline")
                Lines(
                    events.sortedByDescending { it.at }.take(TIMELINE).map { e ->
                        listOfNotNull(ago(e.at, now), listOfNotNull(e.action, e.exitCode?.toString(), e.signal, e.health).joinToString(" "))
                            .joinToString(" · ")
                    },
                )
            }
            more?.healthLog?.takeIf { it.isNotEmpty() }?.let { probes ->
                Section("Health")
                Lines(probes.reversed().map { p -> listOfNotNull(p.exitCode?.let { "exit $it" }, p.output.trim().ifEmpty { null }).joinToString(" · ") }, mono = true)
            }
            more?.logs?.takeIf { it.isNotEmpty() }?.let { logs ->
                Section("Logs")
                Lines(logs.takeLast(LOG_LINES).map { it.line }, mono = true)
            }
            more?.env?.takeIf { it.isNotEmpty() }?.let {
                Section("Environment")
                Lines(it, mono = true)
            }
            more?.mounts?.takeIf { it.isNotEmpty() }?.let { mounts ->
                Section("Mounts")
                Lines(mounts.map { m -> "${m.src} → ${m.dst}" + if (m.mode.isNotEmpty()) " (${m.mode})" else "" }, mono = true)
            }
            Box(Modifier.height(18.dp))
        }
    }
}

@Composable
private fun Section(label: String) {
    val tokens = Kampr.tokens
    LabelText(label, tokens.type.micro, tokens.color.mute, Modifier.padding(top = 14.dp, bottom = 6.dp))
}

@Composable
private fun Facts(lines: List<String>) {
    val tokens = Kampr.tokens
    Column(Modifier.padding(top = 12.dp), verticalArrangement = Arrangement.spacedBy(3.dp)) {
        for (line in lines) KText(line, tokens.type.captionSmall, tokens.color.dim, maxLines = 2)
    }
}

@Composable
private fun Lines(lines: List<String>, mono: Boolean = false) {
    val tokens = Kampr.tokens
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        for (line in lines) KText(line, if (mono) tokens.type.meta else tokens.type.captionSmall, tokens.color.text, maxLines = 4)
    }
}

@Composable
private fun Usage(stats: DockerStats, limit: Long?) {
    Section("Usage")
    val memLimit = stats.memLimit?.takeIf { it > 0 } ?: limit
    Lines(
        listOfNotNull(
            stats.cpuPct?.let { "CPU ${(it * 10).toLong() / 10.0}%" },
            stats.memUsed?.let { used -> "Memory ${formatBytes(used)}" + (memLimit?.let { " of ${formatBytes(it)}" } ?: ", no limit") },
            stats.pids?.let { "$it processes" },
        ),
    )
}
