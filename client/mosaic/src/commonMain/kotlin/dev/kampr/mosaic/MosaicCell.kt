package dev.kampr.mosaic

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.GlyphAction
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.PaneChrome
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.StatusMark
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.readingOrder
import dev.kampr.shared.ui.statusColor
import dev.kampr.shared.ui.statusWord
import dev.kampr.shared.util.formatLatency
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs

val CELL_HEADER = 33.dp
private val FOCUS_EDGE = 2.dp

// A peer this slow reads differently from a local pane whether or not you look at the number,
// which is the whole point of putting the number in the cell.
private const val SLOW_MS = 150.0

private fun cellStatusWord(status: AgentStatus): String? =
    if (status == AgentStatus.Unknown) null else statusWord(status).uppercase()

// Input reaches exactly one cell. Refusing it the way a read-only device is refused it means the
// key row and the destructive-command guard both follow the focus without knowing about it.
internal class CellIo(private val base: PaneIo, private val writable: Boolean) : PaneIo {
    override fun send(msg: ClientMsg) = base.send(msg)
    override fun prefs(paneId: String): PanePrefs = base.prefs(paneId)
    override fun info(paneId: String): PaneInfo? = base.info(paneId)
    override val readOnly: Boolean get() = !writable || base.readOnly
    override fun show(view: PaneView) = base.show(view)
}

@Composable
private fun latencyTone(node: NodeInfo?): Color {
    val color = Kampr.tokens.color
    val rtt = node?.rttMs
    return when {
        node == null || !node.online -> color.blocked
        rtt == null -> color.mute
        rtt >= SLOW_MS -> color.working
        node.kind == "local" -> color.mute
        else -> color.dim
    }
}

private fun status(info: PaneInfo?): AgentStatus = info?.let(::statusOf) ?: AgentStatus.Unknown

fun cellPlace(info: PaneInfo?, node: NodeInfo?, pane: PaneState): String {
    val where = node?.let { "${it.host} / ${it.session}" } ?: info?.nodeId ?: "—"
    // A pane nobody has watched has no measured width, and the node omits `cols` rather than
    // reporting the layout rect, which is a width no row was ever wrapped at.
    val size = info?.let { "${it.cols?.toString() ?: "—"}×${it.rows}" }
        ?: "${pane.cells.cols}×${pane.cells.rows}"
    return "$where · $size"
}

@Composable
fun MosaicCell(
    pane: PaneState,
    info: PaneInfo?,
    node: NodeInfo?,
    focused: Boolean,
    surfaces: PaneSurfaces,
    onFocus: () -> Unit,
    onRemove: () -> Unit,
    modifier: Modifier = Modifier,
    header: Dp = CELL_HEADER,
) {
    val tokens = Kampr.tokens
    val base = LocalPaneIo.current
    val io = remember(base, focused) { CellIo(base, focused) }
    val offline = node != null && !node.online

    Box(
        modifier
            // The surface is taller and wider than the cell by design, and a graphicsLayer does
            // not clip: without this a cell paints over its neighbour and over the chrome.
            .clipToBounds()
            .background(tokens.color.surface2)
            .edge(BorderSpec(FOCUS_EDGE, if (focused) tokens.color.accent else tokens.color.surface2), RectangleShape)
            // Initial pass, unconsumed: focus follows the touch that the terminal underneath is
            // also entitled to act on.
            .pointerInput(Unit) {
                awaitPointerEventScope {
                    while (true) {
                        awaitPointerEvent(PointerEventPass.Initial).changes
                            .firstOrNull { it.pressed }
                            ?.let { onFocus() }
                    }
                }
            },
    ) {
        CompositionLocalProvider(
            LocalPaneIo provides io,
            LocalPaneChrome provides PaneChrome(header),
        ) {
            surfaces.Terminal(pane, info, Modifier.fillMaxSize())
        }

        // Under the header, not over it: the cell is unreachable, its controls are not.
        if (offline) CellUnavailable(node.name, node.detail ?: "${node.name} is not connected")

        CellHeader(pane, info, node, focused, onRemove, Modifier.align(Alignment.TopStart).height(header))
    }
}

@Composable
private fun CellHeader(
    pane: PaneState,
    info: PaneInfo?,
    node: NodeInfo?,
    focused: Boolean,
    onRemove: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val status = info?.let(::statusOf) ?: AgentStatus.Unknown
    val quiet = status == AgentStatus.Idle || status == AgentStatus.Unknown
    val title = info?.let(::paneTitle) ?: pane.id
    Row(
        modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .readingOrder(-1f)
            .padding(start = 12.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        StatusMark(status, 7.dp)
        KText(
            title,
            tokens.type.cardTitle,
            if (quiet) tokens.color.dim else tokens.color.text,
        )
        KText(cellPlace(info, node, pane), tokens.type.meta, tokens.color.mute, Modifier.weight(1f))
        node?.rttMs?.let { KText(formatLatency(it), tokens.type.meta, latencyTone(node)) }
        if (pane.stale) KText("STALE", tokens.type.metaSmall, tokens.color.working)
        statusWord(status)?.let {
            KText(it, tokens.type.metaSmall, if (quiet) tokens.color.mute else statusColor(status))
        }
        GlyphAction(
            KamprIcons.cross,
            "Remove $title from the mosaic",
            if (focused) tokens.color.dim else tokens.color.mute,
            28.dp,
            chip = 20.dp,
            onClick = onRemove,
        )
    }
}

// The mesh already degrades one node without touching the others; a cell says so in the node's
// own words and clears itself when the node comes back.
@Composable
private fun CellUnavailable(name: String, detail: String) {
    val tokens = Kampr.tokens
    Box(
        Modifier
            .fillMaxSize()
            .background(tokens.color.bg.copy(alpha = UNAVAILABLE_WASH))
            .announce("$name is unavailable. $detail. It recovers on its own."),
        contentAlignment = Alignment.Center,
    ) {
        Surface(
            Modifier.padding(horizontal = 18.dp),
            background = tokens.color.blockedBg,
            radius = tokens.radii.md,
            border = BorderSpec(1.dp, tokens.color.blocked),
        ) {
            Column(
                Modifier.padding(horizontal = 16.dp, vertical = 13.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(5.dp),
            ) {
                KText("Unavailable", tokens.type.bodyStrong, tokens.color.blocked)
                KText(detail, tokens.type.caption, tokens.color.dim, maxLines = 3)
                KText("recovers on its own", tokens.type.micro, tokens.color.mute)
            }
        }
    }
}

private const val UNAVAILABLE_WASH = 0.78f
