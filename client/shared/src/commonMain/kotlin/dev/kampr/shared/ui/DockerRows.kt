package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ADOPTION_FAILING
import dev.kampr.shared.model.Fold
import dev.kampr.shared.model.NodeGroup
import dev.kampr.shared.model.byProject
import dev.kampr.shared.model.childOf
import dev.kampr.shared.model.containerPhrase
import dev.kampr.shared.model.containerSpoken
import dev.kampr.shared.model.shelf
import dev.kampr.shared.model.shellsOf
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ContainerInfo

fun nowSeconds(nowMillis: Double): Long = (nowMillis / 1000).toLong()

fun NodeGroup.listedContainers(stoppedOpen: Boolean, nowMillis: Double): List<ContainerInfo> {
    val shelf = shelf(nowSeconds(nowMillis))
    return if (stoppedOpen) shelf.shown + shelf.stopped else shelf.shown
}

@Composable
fun ContainerSection(group: NodeGroup, ctx: HerdRowsCtx) {
    val node = group.node
    val tokens = Kampr.tokens
    val look = ctx.look
    val stoppedOpen = ctx.folds.open(node.id, Fold.Stopped)
    if (node.docker == false) {
        KText("docker is not answering here", tokens.type.captionSmall, tokens.color.mute, Modifier.padding(start = look.inset))
    }
    for (project in group.listedContainers(stoppedOpen, ctx.now).byProject()) {
        project.project?.let { ProjectCaption(it, ctx) }
        for (container in project.containers) {
            ContainerRow(node.id, container, ctx)
            val shells = group.shellsOf(container)
            val child = group.childOf(container)
            if (shells.isNotEmpty() || child != null) {
                Nested(look.gap) {
                    for (pane in shells) PaneItem(pane, ctx)
                    child?.let { ChildNode(it, ctx) }
                }
            }
        }
    }
    val stopped = group.shelf(nowSeconds(ctx.now)).stopped.size
    if (stopped > 0) StoppedRow(stopped, stoppedOpen, ctx) { ctx.onFold(node.id, Fold.Stopped, !stoppedOpen) }
}

@Composable
private fun ProjectCaption(project: String, ctx: HerdRowsCtx) {
    val tokens = Kampr.tokens
    KText(
        project,
        tokens.type.micro,
        tokens.color.mute,
        Modifier.padding(start = ctx.look.inset, top = 4.dp).asHeading().named("Project $project"),
    )
}

@Composable
private fun StoppedRow(count: Int, open: Boolean, ctx: HerdRowsCtx, onToggle: () -> Unit) {
    val tokens = Kampr.tokens
    Row(
        Modifier
            .fillMaxWidth()
            .touchable(LANDSCAPE_TOUCH)
            .action("$count stopped containers", onToggle, state = if (open) "expanded" else "collapsed")
            .padding(start = ctx.look.inset),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        DisableSelection { KText(if (open) "hide $count stopped" else "+$count stopped", tokens.type.captionSmall, tokens.color.dim) }
    }
}

@Composable
fun containerColor(container: ContainerInfo): Color {
    val color = Kampr.tokens.color
    return when {
        container.health == "unhealthy" || container.adoptError != null -> color.blocked
        container.state == "running" -> color.done
        container.state == "restarting" -> color.working
        container.state in setOf("exited", "dead") && (container.exitCode ?: 0) != 0 -> color.blocked
        else -> color.idle
    }
}

fun containerShape(container: ContainerInfo): MarkShape = when {
    container.health == "unhealthy" || container.adoptError != null -> MarkShape.Square
    container.state == "running" -> MarkShape.Bar
    container.state == "restarting" -> MarkShape.Circle
    else -> MarkShape.Ring
}

@Composable
fun ContainerRow(nodeId: String, container: ContainerInfo, ctx: HerdRowsCtx) {
    val tokens = Kampr.tokens
    val manage = LocalManage.current
    val shape = RoundedCornerShape(tokens.radii.sm)
    val spoken = containerSpoken(container, ctx.now)
    Row(
        Modifier
            .fillMaxWidth()
            .touchable(if (ctx.look.rows) LANDSCAPE_TOUCH else TOUCH)
            .let { if (manage.enabled) it.action(spoken, { manage.openContainer(nodeId, container.id) }, shape) else it.named(spoken) }
            .padding(start = 10.dp, end = 4.dp, top = 2.dp, bottom = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Mark(containerColor(container), containerShape(container), 7.dp)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(1.dp)) {
            KText(container.name, tokens.type.cardTitleQuiet, tokens.color.text, overflow = ELIDE_MIDDLE)
            KText(containerPhrase(container, ctx.now), tokens.type.micro, tokens.color.mute)
        }
        if (container.restarts > 0) KText("↻${container.restarts}", tokens.type.micro, tokens.color.working)
        if (container.health == "unhealthy") KText("unhealthy", tokens.type.micro, tokens.color.blocked)
        if (container.adoptError != null) KText(ADOPTION_FAILING, tokens.type.micro, tokens.color.blocked)
        if (manage.enabled && container.state == "running") {
            GlyphAction(KamprIcons.plus, "Shell into ${container.name}", tokens.color.accent, ctx.look.control, chip = 22.dp) {
                manage.containerShell(nodeId, container.id)
            }
        }
    }
}
