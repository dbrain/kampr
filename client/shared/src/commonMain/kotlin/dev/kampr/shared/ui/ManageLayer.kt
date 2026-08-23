package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import dev.kampr.shared.model.Herd
import dev.kampr.shared.theme.Kampr

// `caps.manage` false, or a read-only device, means the affordance is absent rather than
// present-and-failing — so every entry point asks this before it draws anything.
interface ManageIo {
    val enabled: Boolean
    fun openNew(paneId: String?)
    fun openActions(paneId: String)
}

private object NoManage : ManageIo {
    override val enabled: Boolean get() = false
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
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

@Composable
fun PaneManageAction(paneId: String, target: Dp = TOUCH, modifier: Modifier = Modifier) {
    val manage = LocalManage.current
    if (!manage.enabled) return
    GlyphAction(KamprIcons.ellipsis, "Pane actions", Kampr.tokens.color.dim, target, modifier) {
        manage.openActions(paneId)
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
    when (val sheet = state.sheet) {
        null -> Unit
        is Sheet.New -> {
            val node = herd.nodes.firstOrNull { it.id == sheet.nodeId } ?: return
            NewSheet(
                breakpoint = breakpoint,
                node = node,
                pane = sheet.paneId?.let { id -> herd.panes.firstOrNull { it.id == id } },
                nodes = herd.nodes,
                caps = caps[node.id] ?: caps.values.singleOrNull(),
                outcome = outcome,
                onManage = state::manage,
                // A pane belongs to exactly one machine, so aiming the sheet somewhere else is
                // also letting go of the pane it was opened from.
                onNode = { state.openSheet(Sheet.New(it, null)) },
                onNodePicker = { state.go(Screen.Setup) },
                onDismiss = state::closeSheet,
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
            )
        }
    }
}
