package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
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
    GlyphButton(KamprIcons.plus, Kampr.tokens.color.accent, target, modifier) { manage.openNew(paneId) }
}

@Composable
fun PaneManageAction(paneId: String, target: Dp = TOUCH, modifier: Modifier = Modifier) {
    val manage = LocalManage.current
    if (!manage.enabled) return
    GlyphButton(KamprIcons.ellipsis, Kampr.tokens.color.dim, target, modifier) { manage.openActions(paneId) }
}

// 44 dp in portrait and 36 in landscape is the touch rule; the painted chip stays small so the
// header does not grow around it, and the target is the box that catches the tap.
private val TOUCH = 44.dp
val LANDSCAPE_TOUCH = 36.dp
private val CHIP = 28.dp

@Composable
private fun GlyphButton(
    icon: Icon,
    tint: Color,
    target: Dp,
    modifier: Modifier,
    onClick: () -> Unit,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Box(modifier.size(target).clickable(onClick = onClick), contentAlignment = Alignment.Center) {
        Box(
            Modifier
                .size(CHIP)
                .background(tokens.color.raise, shape)
                .edge(tokens.card, shape),
            contentAlignment = Alignment.Center,
        ) {
            IconGlyph(icon, 15.dp, tint)
        }
    }
}

@Composable
fun ManageLayer(state: AppState, herd: Herd, breakpoint: Breakpoint) {
    val outcome by state.store.managed.collectAsState()
    val caps by state.store.nodeCaps.collectAsState()
    when (val sheet = state.sheet) {
        null -> Unit
        is Sheet.New -> {
            val node = herd.nodes.firstOrNull { it.id == sheet.nodeId } ?: return
            NewSheet(
                breakpoint = breakpoint,
                node = node,
                pane = sheet.paneId?.let { id -> herd.panes.firstOrNull { it.id == id } },
                peers = herd.nodes.filter { it.id != node.id },
                caps = caps[node.id] ?: caps.values.singleOrNull(),
                outcome = outcome,
                onManage = state::manage,
                onNodePicker = { state.go(Screen.Setup) },
                onDismiss = state::closeSheet,
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
