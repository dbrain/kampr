package dev.kampr.mosaic

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.absolutePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.BottomEdgeHeldBelow
import dev.kampr.shared.ui.Icon
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.Pill
import dev.kampr.shared.ui.Segmented
import dev.kampr.shared.ui.Mark
import dev.kampr.shared.ui.MarkShape
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.asHeading
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.readingOrder
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.ui.edgeBottom
import dev.kampr.shared.ui.edgeTop
import dev.kampr.shared.util.formatLatency

private val GAP = 1.dp

@Composable
fun MosaicScreen(
    store: KamprStore,
    mosaic: MosaicState,
    herd: Herd,
    connectionStatus: ConnectionStatus,
    build: String?,
    surfaces: PaneSurfaces,
    onHerd: () -> Unit,
    onAdd: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        MosaicBar(mosaic, herd, onHerd, onAdd)
        Box(Modifier.weight(1f).fillMaxWidth().background(tokens.color.line)) {
            // The status row below is what ends at the window, so a cell owes nothing at its own
            // bottom edge — and a terminal that paid anyway floated its controls a gesture
            // handle's worth above the grid it belongs to.
            BottomEdgeHeldBelow(held = true) {
                if (mosaic.panes.isEmpty()) {
                    EmptyMosaic(onAdd)
                } else {
                    MosaicGrid(store, mosaic, herd, surfaces)
                }
            }
        }
        MosaicStatus(mosaic, herd, connectionStatus, build)
    }
}

@Composable
private fun MosaicGrid(store: KamprStore, mosaic: MosaicState, herd: Herd, surfaces: PaneSurfaces) {
    val nodes = herd.nodes.associateBy { it.id }
    val drag = remember { MosaicDrag() }
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val shape = mosaicShape(mosaic.panes.size, maxWidth)
        var index = 0
        Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(GAP)) {
            for (row in shape.perRow) {
                Row(Modifier.weight(1f).fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(GAP)) {
                    repeat(row) {
                        val at = index++
                        val paneId = mosaic.panes[at]
                        val info = herd.panes.firstOrNull { it.id == paneId }
                        MosaicCell(
                            pane = store.pane(paneId),
                            info = info,
                            node = nodes[info?.nodeId ?: paneId.substringBefore('/')],
                            focused = mosaic.focused == paneId,
                            surfaces = surfaces,
                            onFocus = { mosaic.focus(paneId) },
                            onRemove = { mosaic.remove(paneId) },
                            modifier = Modifier.weight(1f).fillMaxSize(),
                            drag = drag,
                            place = "cell ${at + 1} of ${mosaic.panes.size}",
                            onDrop = { onto -> mosaic.move(paneId, mosaic.panes.indexOf(onto)) },
                            onMove = { delta -> mosaic.moveBy(paneId, delta) },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun MosaicBar(mosaic: MosaicState, herd: Herd, onHerd: () -> Unit, onAdd: () -> Unit) {
    val tokens = Kampr.tokens
    // Every node in the herd is one herdr server, so sessions is the node count and "nodes" is
    // the number of machines behind them. Counting distinct session *names* would call two hosts
    // both running `default` one session.
    val sessions = herd.nodes.size
    val hosts = herd.nodes.map { it.host }.toSet().size
    val safe = LocalSafeArea.current
    Row(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeBottom()
            .absolutePadding(
                left = 18.dp + safe.left,
                top = 11.dp + safe.top,
                right = 18.dp + safe.right,
                bottom = 11.dp,
            ),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        KText("Kampr", tokens.type.screenTitle, tokens.color.text)
        Segmented(listOf("Herd", "Mosaic"), 1, { if (it == 0) onHerd() }, Modifier.width(168.dp))
        KText(
            listOf(
                count(mosaic.panes.size, "pane"),
                count(hosts, "node"),
                count(sessions, "session"),
            ).joinToString(" · "),
            tokens.type.caption,
            tokens.color.mute,
        )
        Box(Modifier.weight(1f))
        BarButton(KamprIcons.plus, "Add pane", tokens.color.accent, onAdd, enabled = !mosaic.full)
        BarButton(
            null,
            if (mosaic.saved) "Saved on this device" else "Save layout",
            tokens.color.dim,
            mosaic::save,
            enabled = !mosaic.saved,
        )
    }
}

@Composable
private fun BarButton(icon: Icon?, text: String, tint: Color, onClick: () -> Unit, enabled: Boolean) {
    val tokens = Kampr.tokens
    val color = if (enabled) tint else tokens.color.mute
    val shape = RoundedCornerShape(tokens.radii.pill)
    Pill(
        Modifier.let { if (enabled) it.action(text, onClick, shape) else it },
        horizontal = 13.dp,
        vertical = 7.dp,
    ) {
        if (icon != null) IconGlyph(icon, 13.dp, color)
        KText(text, tokens.type.buttonSmall, color)
    }
}

@Composable
private fun EmptyMosaic(onAdd: () -> Unit) {
    val tokens = Kampr.tokens
    Box(Modifier.fillMaxSize().background(tokens.color.surface2), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(9.dp),
        ) {
            KText("Nothing in the mosaic yet", tokens.type.paneTitle, tokens.color.dim, Modifier.asHeading())
            KText(
                "Up to $MAX_CELLS panes, from any session on any node in the herd.",
                tokens.type.caption,
                tokens.color.mute,
            )
            Box(Modifier.height(4.dp))
            BarButton(KamprIcons.plus, "Add pane", tokens.color.accent, onAdd, enabled = true)
        }
    }
}

// Looking at four panes changes nothing anywhere: four `observe` streams, and not one call that
// could reshape a pane. The count is what this client actually holds open, not a decoration.
@Composable
private fun MosaicStatus(
    mosaic: MosaicState,
    herd: Herd,
    connectionStatus: ConnectionStatus,
    build: String?,
) {
    val tokens = Kampr.tokens
    val hub = herd.nodes.firstOrNull { it.kind == "local" }
    val live = connectionStatus is ConnectionStatus.Live
    val safe = LocalSafeArea.current
    Row(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeTop()
            .readingOrder(1f)
            .absolutePadding(
                left = 18.dp + safe.left,
                top = 8.dp,
                right = 18.dp + safe.right,
                bottom = 8.dp + safe.bottom,
            ),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        Row(
            Modifier.announce(
                if (live) "Connected to hub ${hub?.name ?: "unknown"}"
                else "Not connected to hub ${hub?.name ?: "unknown"}",
            ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(7.dp),
        ) {
            Mark(
                if (live) tokens.color.done else tokens.color.working,
                if (live) MarkShape.Bar else MarkShape.Ring,
                6.dp,
            )
            KText(
                "hub · ${hub?.name ?: "—"}",
                tokens.type.meta,
                if (live) tokens.color.done else tokens.color.working,
            )
        }
        for (node in herd.nodes.filter { it.kind != "local" }) {
            val skewed = node.build != null && build != null && node.build != build
            KText(
                listOfNotNull(
                    node.name,
                    if (node.online) formatLatency(node.rttMs) else "offline",
                    node.build.takeIf { skewed },
                ).joinToString(" "),
                tokens.type.meta,
                when {
                    !node.online -> tokens.color.blocked
                    skewed -> tokens.color.working
                    else -> tokens.color.mute
                },
                Modifier.announce(
                    when {
                        !node.online -> "${node.name} is offline"
                        skewed -> "${node.name} is on build ${node.build}, which differs from this client"
                        else -> "${node.name}, ${formatLatency(node.rttMs)}"
                    },
                ),
            )
        }
        Box(Modifier.weight(1f))
        KText(
            "${mosaic.observers} observers · 0 panes reshaped",
            tokens.type.meta,
            tokens.color.mute,
            Modifier.named("${mosaic.observers} observe streams open, no panes reshaped"),
        )
        KText("kampr ${build ?: "0.1.0"}", tokens.type.meta, tokens.color.mute)
    }
}

private fun count(n: Int, noun: String): String = if (n == 1) "1 $noun" else "$n ${noun}s"
