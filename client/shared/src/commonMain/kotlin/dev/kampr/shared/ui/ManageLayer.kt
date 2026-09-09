package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.capsFor
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.NodeInfo

// Root-window coordinates for the corner a context menu hangs off, which is the pointer's own cell
// on a desk (#426) and the "…" glyph's own box when a finger asked. Null on a phone, where the menu
// is a bottom sheet and there is no pointer to anchor to.
data class MenuAnchor(val x: Dp, val y: Dp)

// `caps.manage` false, or a read-only device, means the affordance is absent rather than
// present-and-failing — so every entry point asks this before it draws anything.
interface ManageIo {
    val enabled: Boolean
    fun openNew(paneId: String?)
    // The in-session sheet: the pane, its tab and its workspace, and the things that only mean
    // something while you are looking at the pane.
    fun openActions(paneId: String)
    // The list menu, on a surface where the pane is a row rather than a screen. Defaulted onto
    // `openActions` because `ManageIo` is implemented by test doubles in four modules and two of
    // them are not this one's to edit; `AppManage` is the only implementor that answers it.
    fun openMenu(paneId: String, at: MenuAnchor? = null) = openActions(paneId)

    // The + beside one machine: a workspace on that host, with nothing to fill in. There is no
    // sheet in front of it, so the ack and the refusal are both `AppState`'s to catch.
    fun quickWorkspace(nodeId: String) = Unit
}

private object NoManage : ManageIo {
    override val enabled: Boolean get() = false
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
    override fun openMenu(paneId: String, at: MenuAnchor?) = Unit
    override fun quickWorkspace(nodeId: String) = Unit
}

val LocalManage: ProvidableCompositionLocal<ManageIo> = staticCompositionLocalOf { NoManage }

class AppManage(private val state: AppState) : ManageIo {
    override val enabled: Boolean get() = state.store.canManage
    override fun openNew(paneId: String?) {
        val node = paneId?.let { state.store.paneInfo(it)?.nodeId }
            ?: state.store.herd.value.nodes.firstOrNull { it.kind == "local" }?.id
            ?: state.store.herd.value.nodes.firstOrNull()?.id
            ?: return
        state.openSheet(Sheet.New(node, paneId))
    }

    override fun openActions(paneId: String) {
        state.openSheet(Sheet.Actions(paneId))
    }

    override fun openMenu(paneId: String, at: MenuAnchor?) {
        state.openSheet(Sheet.Menu(paneId, at))
    }

    override fun quickWorkspace(nodeId: String) {
        state.quickWorkspace(nodeId)
    }
}

@Composable
fun NewAction(paneId: String? = null, target: Dp = TOUCH, modifier: Modifier = Modifier) {
    val manage = LocalManage.current
    if (!manage.enabled) return
    GlyphAction(
        KamprIcons.plus,
        if (paneId == null) "New workspace or session" else "New, from this pane",
        Kampr.tokens.color.accent,
        target,
        modifier,
    ) { manage.openNew(paneId) }
}

// The host's own +, beside the machine it makes something on. The bar's + opens the sheet — the
// machine picker, the label, the directory, the variables, the worktrees, the named sessions — and
// that is the right screen for all of those and the wrong one for "a terminal on that box", which
// is the thing asked for most and the thing with nothing in it to decide.
//
// Asked of `isReachable` rather than of `online`: a manage op is served by the node, and the node
// starts a stopped herdr for one (#324, #325), so the host this is most use on is the cold one.
@Composable
fun QuickWorkspaceAction(node: NodeInfo, target: Dp = TOUCH, modifier: Modifier = Modifier) {
    val manage = LocalManage.current
    if (!manage.enabled || !node.isReachable) return
    GlyphAction(
        KamprIcons.plus,
        "New workspace on ${node.name}",
        Kampr.tokens.color.accent,
        target,
        modifier,
        chip = 22.dp,
    ) { manage.quickWorkspace(node.id) }
}

@Composable
fun PaneManageAction(paneId: String, target: Dp = TOUCH, modifier: Modifier = Modifier) {
    val manage = LocalManage.current
    if (!manage.enabled) return
    GlyphAction(KamprIcons.ellipsis, "Pane actions", Kampr.tokens.color.dim, target, modifier) {
        manage.openActions(paneId)
    }
}

// The list menu's own way in. Herdr anchors a context menu at the pointer's cell (#426) and a
// glyph has no pointer, so it hangs off the glyph's own bottom-left instead — the same corner, from
// the thing that was pressed.
@Composable
fun PaneMenuAction(paneId: String, target: Dp = TOUCH, modifier: Modifier = Modifier) {
    val manage = LocalManage.current
    if (!manage.enabled) return
    val density = LocalDensity.current
    var anchor by remember(paneId) { mutableStateOf<MenuAnchor?>(null) }
    GlyphAction(
        KamprIcons.ellipsis,
        "Pane menu",
        Kampr.tokens.color.dim,
        target,
        modifier.onGloballyPositioned {
            val box = it.boundsInRoot()
            anchor = with(density) { MenuAnchor(box.left.toDp(), box.bottom.toDp()) }
        },
    ) {
        manage.openMenu(paneId, anchor)
    }
}

@Composable
fun ManageLayer(state: AppState, herd: Herd, breakpoint: Breakpoint) {
    val outcome by state.store.managed.collectAsState()
    val caps by state.store.nodeCaps.collectAsState()
    // A sheet already up when the role moved is a write affordance like any other. `openSheet`
    // refuses to open one for a read-only device; a demotion has to reach the one already open.
    if (!state.store.canManage) {
        LaunchedEffect(Unit) { state.closeSheet() }
        return
    }
    // Not a `Sheet`: the find bar is opened *from* the actions sheet, which closes on the way, and
    // a second entry in that enum would make "which sheet is up" answerable two ways.
    state.findingOn?.let { paneId ->
        val found by state.store.found.collectAsState()
        FindSheet(
            paneId = paneId,
            breakpoint = breakpoint,
            found = found,
            onSearch = { state.find(paneId, it) },
            onDismiss = state::closeFind,
        )
    }
    when (val sheet = state.sheet) {
        null -> Unit
        is Sheet.New -> {
            val node = herd.nodes.firstOrNull { it.id == sheet.nodeId } ?: return
            NewSheet(
                breakpoint = breakpoint,
                node = node,
                pane = sheet.paneId?.let { id -> herd.panes.firstOrNull { it.id == id } },
                nodes = herd.nodes,
                caps = capsFor(node.id, caps, herd),
                outcome = outcome,
                onManage = state::manage,
                // A pane belongs to exactly one machine, so aiming the sheet somewhere else is
                // also letting go of the pane it was opened from.
                onNode = { state.openSheet(Sheet.New(it, null)) },
                onNodePicker = { state.go(Screen.Setup) },
                onDismiss = state::closeSheet,
                onCreated = state::opening,
                panes = herd.panes,
                // `askCaps` keeps a ten-second floor under the connection's own polling, which is
                // right for a herd patch and wrong for the one moment the operator has just
                // changed the answer. This asks past it.
                onRefreshCaps = { state.connection.send(ClientMsg.RequestCaps) },
                agentArgs = state.agentArgs,
            )
        }
        is Sheet.Actions -> {
            val pane = herd.panes.firstOrNull { it.id == sheet.paneId } ?: return
            PaneActionsSheet(
                breakpoint = breakpoint,
                pane = pane,
                outcome = outcome,
                onManage = state::manage,
                onDismiss = state::closeSheet,
                panes = herd.panes,
                // Absent rather than present-and-failing, the same rule the passkey button follows.
                onFind = { state.openFind(pane.id) }
                    .takeIf { state.store.hello.value?.caps?.find == true },
            )
        }
        is Sheet.Menu -> {
            val pane = herd.panes.firstOrNull { it.id == sheet.paneId } ?: return
            PaneMenu(
                breakpoint = breakpoint,
                pane = pane,
                anchor = sheet.at,
                outcome = outcome,
                onManage = state::manage,
                onOpen = { state.openPane(pane.id) },
                onDismiss = state::closeSheet,
            )
        }
    }
}
