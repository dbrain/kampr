package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.Area
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Fold
import dev.kampr.shared.model.Folds
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.NodeGroup
import dev.kampr.shared.model.childOf
import dev.kampr.shared.model.containerSpoken
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.sections
import dev.kampr.shared.model.shellsOf
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.model.strayChildren
import dev.kampr.shared.model.tree
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ContainerInfo
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo

val RAIL_WIDTH: Dp = 56.dp

private val TILE = 44.dp

// Two characters is what a 56 dp rail has room for, and the operator's ask was to "roughly know
// what you're switching to" — so it comes off the same name the expanded row shows rather than
// off the pane id, and takes the tab with it because two panes in one workspace are otherwise
// the same two letters. `paneTitle` is "{identity} · {command}"; only the identity half names
// the pane.
private fun paneSigil(pane: PaneInfo): String {
    val identity = paneTitle(pane).substringBefore(" · ").filter { it.isLetterOrDigit() }
    val tab = pane.tab?.filter { it.isLetterOrDigit() }.orEmpty()
    val head = identity.take(1).uppercase()
    return when {
        head.isEmpty() -> "?"
        tab.isNotEmpty() -> head + tab.take(1).lowercase()
        else -> head + identity.drop(1).take(1).lowercase()
    }
}

// The sidebar with its words taken away, and nothing else. Every pane the expanded list holds is
// still here, still says what it is doing, and still opens — a rail that only kept the room would
// have cost the operator the thing the sidebar is for.
@Composable
fun HerdRail(
    herd: Herd,
    connection: ConnectionStatus,
    now: Double,
    activePaneId: String?,
    deviceName: String,
    deviceDetail: String,
    onOpenPane: (String) -> Unit,
    onSettings: () -> Unit,
    onExpand: () -> Unit,
    modifier: Modifier = Modifier,
    folds: Folds = Folds(),
    onFold: OnFold = NO_FOLD,
) {
    val tokens = Kampr.tokens
    val settings = "Settings — $deviceName, $deviceDetail"
    Column(
        modifier
            .width(RAIL_WIDTH)
            .fillMaxHeight()
            .background(tokens.color.bar)
            .readingOrder(-1f)
            .edgeEnd(),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Column(
            Modifier.padding(top = 14.dp, bottom = 10.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            GlyphAction(
                KamprIcons.chevronRight,
                "Expand the sidebar",
                tokens.color.dim,
                LANDSCAPE_TOUCH,
            ) { onExpand() }
            PaletteAction(LANDSCAPE_TOUCH)
            NewAction(target = LANDSCAPE_TOUCH)
            MosaicAction(LANDSCAPE_TOUCH)
            FleetAction(LANDSCAPE_TOUCH)
            // The pill's mark without the pill: with no strip under the desk, a folded sidebar is
            // otherwise the one place a dropped socket goes unsaid.
            Mark(
                connectionColor(connection),
                connectionShape(connection),
                7.dp,
                Modifier.announce(connectionWord(connection) ?: "Connected"),
            )
        }
        Column(
            Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            val ctx = HerdRowsCtx(HerdLook.Sidebar, now, null, folds, onFold, onOpenPane, activePaneId)
            for (group in herd.tree()) RailGroup(group, ctx)
        }
        Box(
            Modifier
                .fillMaxWidth()
                .edgeTop()
                .padding(bottom = LocalSafeArea.current.bottom)
                .height(TILE)
                .action(settings, onSettings),
            contentAlignment = Alignment.Center,
        ) {
            IconGlyph(KamprIcons.gear, 15.dp, tokens.color.mute)
            Hint(settings)
        }
    }
}

// The folds and the picked list are the sidebar's, read here and never set: a rail has no room for
// the switch.
@Composable
private fun RailGroup(group: NodeGroup, ctx: HerdRowsCtx) {
    val node = group.node.id
    val pane: @Composable (PaneInfo) -> Unit = { RailPane(it, ctx.now, it.id == ctx.activePaneId) { ctx.onOpenPane(it.id) } }
    RailNodeMark(group.node)
    when (ctx.folds.area(group)) {
        Area.Terminals -> for ((index, section) in group.sections.withIndex()) {
            // No room for a name: a gap between workspaces, so two of them do not read as one.
            if (index > 0 && section.heading != null) Box(Modifier.height(6.dp).named("Workspace ${section.heading}"))
            for (p in section.panes) pane(p)
        }

        Area.Muffit -> for (p in group.background) pane(p)

        Area.Containers -> for (container in group.listedContainers(ctx.folds.open(node, Fold.Stopped), ctx.now)) {
            RailContainer(node, container, ctx.now)
            for (p in group.shellsOf(container)) pane(p)
            group.childOf(container)?.let { RailGroup(it, ctx) }
        }
    }
    for (child in group.strayChildren) RailGroup(child, ctx)
}

@Composable
private fun RailContainer(nodeId: String, container: ContainerInfo, now: Double) {
    val tokens = Kampr.tokens
    val manage = LocalManage.current
    val shape = RoundedCornerShape(tokens.radii.sm)
    val spoken = containerSpoken(container, now)
    Box(
        Modifier
            .padding(horizontal = 5.dp)
            .fillMaxWidth()
            .height(TILE)
            .let { if (manage.enabled) it.action(spoken, { manage.openContainer(nodeId, container.id) }, shape) else it.named(spoken) },
        contentAlignment = Alignment.Center,
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(3.dp)) {
            KText(containerSigil(container.name), tokens.type.pill, tokens.color.dim, maxLines = 1)
            Mark(containerColor(container), containerShape(container), 7.dp)
        }
        if (manage.enabled) Hint(spoken)
    }
}

private fun containerSigil(name: String): String {
    val letters = (name.substringAfterLast('/').split('-', '_', '.').lastOrNull { it.any(Char::isLetter) } ?: name).filter { it.isLetterOrDigit() }
    return letters.take(1).uppercase() + letters.drop(1).take(1).lowercase()
}

// A machine is a separator here rather than a header: the rail has no room for a name, and a run
// of panes with nothing between them would read as one machine's.
@Composable
private fun RailNodeMark(node: NodeInfo, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    Box(
        modifier.fillMaxWidth().height(18.dp).named(nodeReach(node).let { "${node.name}, $it" }),
        contentAlignment = Alignment.Center,
    ) {
        Mark(
            if (node.isServing) tokens.color.dim else tokens.color.blocked,
            if (node.isServing) MarkShape.Bar else MarkShape.Ring,
            10.dp,
        )
    }
}

@Composable
private fun RailPane(pane: PaneInfo, now: Double, active: Boolean, onClick: () -> Unit) {
    val tokens = Kampr.tokens
    val status = statusOf(pane)
    val quiet = status == AgentStatus.Idle || status == AgentStatus.Unknown
    val shape = RoundedCornerShape(tokens.radii.sm)
    val manage = LocalManage.current
    val spoken = "Open ${paneSpoken(pane, now)}"
    Box(
        Modifier
            .padding(horizontal = 5.dp)
            .fillMaxWidth()
            .height(TILE)
            .let { if (active) it.background(tokens.color.raise, shape) else it }
            .paneMenu(pane.id)
            .action(
                spoken,
                onClick,
                shape,
                role = Role.Tab,
                selected = active,
                onLongClick = if (manage.enabled) ({ manage.openMenu(pane.id) }) else null,
            ),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            KText(
                paneSigil(pane),
                tokens.type.pill,
                if (quiet) tokens.color.dim else tokens.color.text,
                maxLines = 1,
            )
            Box(Modifier.size(7.dp), contentAlignment = Alignment.Center) {
                StatusMark(status, 7.dp)
            }
        }
        Hint(spoken, byTouch = !manage.enabled)
    }
}
