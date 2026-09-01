package dev.kampr.mosaic

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.PaneGone
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.WatchPresence
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.model.watchersTag
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.GlyphAction
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.PaneChrome
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneManageAction
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.StreamNotice
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.WatchNotice
import dev.kampr.shared.ui.rememberWatchPresence
import dev.kampr.shared.ui.StatusMark
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.paneActions
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
    gone: PaneGone? = null,
    header: Dp = CELL_HEADER,
    drag: MosaicDrag? = null,
    place: String = "",
    onDrop: (String) -> Unit = {},
    onMove: (Int) -> Unit = {},
) {
    val tokens = Kampr.tokens
    val base = LocalPaneIo.current
    val io = remember(base, focused) { CellIo(base, focused) }
    val offline = node != null && !node.online
    val held = drag?.held == pane.id
    val presence = rememberWatchPresence(pane.id, info)

    DisposableEffect(drag, pane.id) { onDispose { drag?.forget(pane.id) } }

    Box(
        modifier
            // The surface is taller and wider than the cell by design, and a graphicsLayer does
            // not clip: without this a cell paints over its neighbour and over the chrome.
            .clipToBounds()
            // Right-click only, which is all `paneActions` is: a long press inside a cell is the
            // grid's, exactly as it is on the pane screen, and the header carries the touch
            // affordance. The cell claims no `onLongClick` of its own, which is the opt-out.
            .paneActions(pane.id)
            // A drag needs to know where the cells actually landed, and only the layout does.
            .onGloballyPositioned {
                val rect = it.boundsInWindow()
                drag?.place(pane.id, rect.left, rect.top, rect.right, rect.bottom)
            }
            .background(tokens.color.surface2)
            .edge(
                BorderSpec(
                    FOCUS_EDGE,
                    when {
                        held -> tokens.color.accentHi
                        focused -> tokens.color.accent
                        else -> tokens.color.surface2
                    },
                ),
                RectangleShape,
            )
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
        if (offline) {
            CellUnavailable(node.name, node.detail ?: "${node.name} is not connected")
        } else {
            // A node can be online and still unable to stream — the socket answers and the
            // binary that carries the screens does not. A blank cell is the same lie here as it
            // is on the pane screen, and the wording it needs is the one the node already sent.
            streamFault(pane, info)?.let { CellNoStream(it) }
        }

        CellHeader(
            pane, info, node, gone, presence, focused, onRemove, drag, place, onDrop, onMove,
            Modifier.align(Alignment.TopStart).height(header),
        )

        // Four cells announcing at once is four interruptions for one fact, so only the cell that
        // input actually reaches puts the notice up. The others carry it in their header.
        if (focused) {
            WatchNotice(
                presence,
                Modifier.align(Alignment.TopEnd).padding(top = header + 6.dp, end = 10.dp),
            )
        }
    }
}

@Composable
private fun CellHeader(
    pane: PaneState,
    info: PaneInfo?,
    node: NodeInfo?,
    gone: PaneGone?,
    presence: WatchPresence,
    focused: Boolean,
    onRemove: () -> Unit,
    drag: MosaicDrag?,
    place: String,
    onDrop: (String) -> Unit,
    onMove: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val status = info?.let(::statusOf) ?: AgentStatus.Unknown
    val quiet = status == AgentStatus.Idle || status == AgentStatus.Unknown
    // Never the whole id. A pane that has left the herd took its name with it, and a node ULID in
    // the slot the name was in reads as the mosaic having lost its place rather than as the shell
    // having finished. `CLOSED` beside it is what says which.
    val title = info?.let(::paneTitle) ?: pane.id.substringAfter('/')
    Row(
        modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .readingOrder(-1f)
            .padding(start = 4.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        // The header is the handle, never the surface: dragging the grid itself would be the same
        // gesture as panning it, and the pan is what a terminal is for.
        DragGrip(title, place, drag, pane.id, onDrop, onMove)
        StatusMark(status, 7.dp)
        KText(
            title,
            tokens.type.cardTitle,
            if (quiet) tokens.color.dim else tokens.color.text,
        )
        KText(cellPlace(info, node, pane), tokens.type.meta, tokens.color.mute, Modifier.weight(1f))
        node?.rttMs?.let { KText(formatLatency(it), tokens.type.meta, latencyTone(node)) }
        when (gone) {
            PaneGone.Shell -> KText("CLOSED", tokens.type.metaSmall, tokens.color.blocked)
            PaneGone.Node -> KText("NODE GONE", tokens.type.metaSmall, tokens.color.blocked)
            null -> if (pane.stale) KText("STALE", tokens.type.metaSmall, tokens.color.working)
        }
        watchersTag(presence.others)?.let {
            KText(it.uppercase(), tokens.type.metaSmall, tokens.color.mute)
        }
        statusWord(status)?.let {
            KText(it, tokens.type.metaSmall, if (quiet) tokens.color.mute else statusColor(status))
        }
        PaneManageAction(pane.id, 28.dp)
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

// Why this pane will never paint, when there is no grid on its surface to read instead.
private fun streamFault(pane: PaneState, info: PaneInfo?): String? =
    info?.detail?.takeUnless { pane.painted }

// The same words the pane screen shows, over the same wash the offline cell uses. One notice, two
// placements: a cell that phrased this itself would be a second copy of the one thing on this
// surface an operator has to act on.
@Composable
private fun CellNoStream(detail: String) {
    val tokens = Kampr.tokens
    Box(
        Modifier.fillMaxSize().background(tokens.color.bg.copy(alpha = UNAVAILABLE_WASH)),
        contentAlignment = Alignment.Center,
    ) {
        StreamNotice(detail, Modifier.padding(horizontal = 14.dp))
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

// Three ways in, because a drag is one and the readers this mosaic also has to serve reach none of
// it: a pointer drags it, Tab-and-arrows moves it, and a screen reader lists the two moves.
@Composable
private fun DragGrip(
    title: String,
    place: String,
    drag: MosaicDrag?,
    paneId: String,
    onDrop: (String) -> Unit,
    onMove: (Int) -> Unit,
) {
    val tokens = Kampr.tokens
    var origin by remember(paneId) { mutableStateOf(Offset.Zero) }
    var focused by remember { mutableStateOf(false) }
    val tint = if (drag?.held == paneId) tokens.color.accent else tokens.color.mute
    Column(
        Modifier
            .size(width = 20.dp, height = CELL_HEADER)
            .then(if (focused) Modifier.border(2.dp, tokens.color.accentHi) else Modifier)
            .onGloballyPositioned { origin = it.positionInWindow() }
            .semantics(mergeDescendants = true) {
                contentDescription = "Reorder $title"
                role = Role.Button
                if (place.isNotEmpty()) stateDescription = place
                customActions = listOf(
                    CustomAccessibilityAction("Move this pane earlier") { onMove(-1); true },
                    CustomAccessibilityAction("Move this pane later") { onMove(1); true },
                )
            }
            .onFocusChanged { focused = it.isFocused }
            .focusable()
            .onPreviewKeyEvent { event ->
                if (event.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                val delta = when (event.key) {
                    Key.DirectionLeft, Key.DirectionUp -> -1
                    Key.DirectionRight, Key.DirectionDown -> 1
                    else -> return@onPreviewKeyEvent false
                }
                onMove(delta)
                true
            }
            .pointerInput(paneId, drag) {
                if (drag == null) return@pointerInput
                detectDragGestures(
                    onDragStart = { drag.start(paneId) },
                    onDragEnd = { drag.end() },
                    onDragCancel = { drag.end() },
                ) { change, _ ->
                    change.consume()
                    val point = origin + change.position
                    drag.drag(point.x, point.y)?.let { if (it != paneId) onDrop(it) }
                }
            },
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        repeat(2) { IconGlyph(KamprIcons.ellipsis, 11.dp, tint) }
    }
}

private const val UNAVAILABLE_WASH = 0.78f
