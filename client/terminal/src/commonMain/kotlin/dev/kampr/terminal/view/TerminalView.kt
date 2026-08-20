package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.terminalPalette
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.breakpointOf
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.terminal.PaneSession
import dev.kampr.terminal.guard.SubmitGuard
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.PaneTextInput
import dev.kampr.terminal.render.GridRenderer
import dev.kampr.terminal.render.ModeSelector
import dev.kampr.terminal.render.LogicalText
import dev.kampr.terminal.render.ResolvedStyles
import dev.kampr.terminal.render.Selection
import dev.kampr.terminal.render.Target
import dev.kampr.terminal.render.TargetKind
import dev.kampr.terminal.render.detectTarget
import dev.kampr.terminal.render.SurfaceRows
import dev.kampr.terminal.render.TextCache
import kotlinx.coroutines.delay
import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.min

private const val CURSOR_BLINK_MS = 530L
private const val INDICATOR_HEIGHT_DP = 22f
private const val DECAY = 0.94f
private const val PREFS_DEBOUNCE_MS = 400L
private const val FONT_SETTLE_FRAMES = 120

// Mirrors the header PaneScreen floats over this surface and the answer strip it shows while a
// prompt is outstanding. Chrome insets the scrollable content; it never insets the paint.
private const val PENDING_BAR_DP = 52f

private fun headerInsetDp(breakpoint: Breakpoint): Float = when (breakpoint) {
    Breakpoint.Desktop -> 56f
    Breakpoint.Landscape -> 44f
    Breakpoint.Portrait -> 108f
}

private fun pendingInsetPx(pane: PaneState, density: Density): Float =
    if (pane.pending != null) with(density) { PENDING_BAR_DP.dp.toPx() } else 0f

@Composable
fun TerminalView(
    pane: PaneState,
    session: PaneSession,
    io: PaneIo,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val palette = remember(tokens) { tokens.terminalPalette() }
    val measurer = rememberTextMeasurer(cacheSize = 0)
    val cache = remember(tokens) { TextCache(measurer, tokens.fonts.terminal) }
    val renderer = remember(cache) { GridRenderer(cache, ModeSelector()) }
    val styles = remember(palette) { ResolvedStyles(palette) }
    val rows = remember(pane) { SurfaceRows(pane) }
    val guard = remember(pane, io, session) { SubmitGuard(pane, io, session.confirm) }
    val sink = remember(pane.id, io, session, guard) { InputSink(pane.id, io, session.latches, guard) }
    val logical = remember(rows) { LogicalText(rows) }
    val probe = remember(pane.id) { GridProbe() }
    // LocalClipboard's ClipEntry is constructed from a platform-native object in CMP 1.11.1,
    // so the deprecated ClipboardManager is still the only clipboard reachable from common code.
    @Suppress("DEPRECATION")
    val clipboard = LocalClipboardManager.current
    val uris = LocalUriHandler.current
    val view = session.view
    val ground = palette.background(pane.styles[0])

    var cursorOn by remember { mutableStateOf(true) }
    LaunchedEffect(pane) {
        while (true) {
            delay(CURSOR_BLINK_MS)
            cursorOn = !cursorOn
        }
    }

    BoxWithConstraints(modifier.fillMaxSize().background(ground)) {
        val density = LocalDensity.current
        val breakpoint = breakpointOf(maxWidth, maxHeight)
        val chromeBottom = session.keyRowHeight + pendingInsetPx(pane, density)
        val paint = PaintRect(
            width = with(density) { maxWidth.toPx() },
            height = with(density) { maxHeight.toPx() },
            insetTop = with(density) { headerInsetDp(breakpoint).dp.toPx() },
            insetBottom = chromeBottom + with(density) { INDICATOR_HEIGHT_DP.dp.toPx() },
        )

        var fontEpoch by remember(cache) { mutableIntStateOf(0) }
        val base = remember(cache, fontEpoch) { cache.metrics(BASE_CELL_SP.sp) }
        val cols = pane.cells.cols
        val presets = remember(paint.width, cols, base) { zoomPresets(paint.width, cols, base.width) }
        val stored = io.prefs(pane.id).zoom

        // Nothing is adopted before the first grid.reset: the placeholder buffer is 80x24 and a
        // zoom taken against it sticks for the life of the pane.
        LaunchedEffect(pane.id, pane.painted, cols, rows.historyRows > 0, paint.width, stored) {
            if (!pane.painted || view.chosen) return@LaunchedEffect
            val fill = defaultZoom(
                paint, cols, rows.liveRows, rows.historyRows, base.width, base.height,
            )
            if (stored != null) view.setZoom(stored, presets) else view.adoptDefault(fill)
            view.scrollY = initialScroll(
                paint, rows.total, rows.historyRows + pane.cursor.row, base.height * view.zoom,
            )
        }
        val zoom = if (view.zoom > 0f) view.zoom else 1f
        val metrics = remember(cache, zoom, fontEpoch) { cache.metrics((BASE_CELL_SP * zoom).sp) }
        LaunchedEffect(cache, zoom) {
            repeat(FONT_SETTLE_FRAMES) {
                withFrameNanos { }
                if (cache.reprobe((BASE_CELL_SP * zoom).sp)) fontEpoch++
            }
        }

        val geometry = terminalGeometry(
            paint, cols, rows.total, metrics.width, metrics.height, view.panX, view.scrollY,
        )
        view.minPanX = geometry.minPanX
        view.maxScroll = geometry.maxScroll

        LaunchedEffect(pane.cursor, view.followCursor, geometry.pinned) {
            if (view.followCursor && geometry.pinned && !view.pinching) {
                view.panX = followCursorPan(
                    view.panX, geometry.minPanX, pane.cursor.col, metrics.width, paint.width,
                )
            }
        }

        LaunchedEffect(view.flings) {
            while (abs(view.velocityX) > 1f || abs(view.velocityY) > 1f) {
                withFrameNanos { }
                view.panX = (view.panX + view.velocityX / 60f).coerceIn(view.minPanX, 0f)
                view.scrollY = (view.scrollY - view.velocityY / 60f).coerceIn(0f, view.maxScroll)
                view.velocityX *= DECAY
                view.velocityY *= DECAY
                if (abs(view.velocityX) < 20f) view.velocityX = 0f
                if (abs(view.velocityY) < 20f) view.velocityY = 0f
            }
        }

        // Only a zoom the operator picked is worth storing; writing the computed default back
        // would freeze it against a later geometry change.
        LaunchedEffect(view.zoom, view.remembered, view.chosen) {
            if (!view.chosen || !view.remembered) return@LaunchedEffect
            delay(PREFS_DEBOUNCE_MS)
            io.send(ClientMsg.SetPrefs(pane.id, mapOf("zoom" to formatZoom(view.zoom))))
        }

        probe.originX = geometry.originX
        probe.originY = geometry.originY
        probe.cellWidth = metrics.width
        probe.cellHeight = metrics.height
        probe.cols = cols.coerceAtLeast(1)
        probe.totalRows = rows.total.coerceAtLeast(1)

        fun tapped(position: Offset) {
            if (view.selection != null) {
                view.selection = null
                view.target = null
                return
            }
            val cell = probe.cellAt(position)
            val declared = pane.links.getOrNull(logical.linkAt(cell.row, cell.col))
            if (declared != null) {
                view.target = Target(declared, TargetKind.Link)
                return
            }
            val (line, offset) = logical.lineAt(cell.row)
            val found = detectTarget(line, offset + cell.col)
            view.target = found
            if (found == null) session.openKeyboard()
        }

        Box(
            Modifier.fillMaxSize().pointerInput(pane.id) {
                terminalGestures(session, presets, paint, probe, ::tapped)
            },
        ) {
            Box(
                Modifier
                    .fillMaxSize()
                    .graphicsLayer {
                        scaleX = view.layerScale
                        scaleY = view.layerScale
                        translationX = view.layerTx
                        translationY = view.layerTy
                        transformOrigin = TransformOrigin(0f, 0f)
                    }
                    .drawBehind {
                        pane.revision
                        styles.sync(pane.styles)
                        renderer.draw(
                            scope = this,
                            rows = rows,
                            styles = styles,
                            cellWidth = metrics.width,
                            cellHeight = metrics.height,
                            originX = geometry.originX,
                            originY = geometry.originY,
                            cursorCol = pane.cursor.col,
                            cursorRow = pane.cursor.row,
                            cursorOn = cursorOn && pane.cursor.visible,
                            selection = view.selection,
                            selectionWash = palette.selectionWash,
                            linkInk = palette.linkInk,
                        )
                        if (pane.stale) drawRect(tokens.color.bg.copy(alpha = 0.45f), size = size)
                    },
            )
        }

        PaneTextInput(
            session = session,
            sink = sink,
            enabled = !io.readOnly,
            modifier = Modifier.align(Alignment.BottomStart).size(1.dp),
        )

        val firstCol = floor(-geometry.panX / metrics.width).toInt().coerceIn(0, cols)
        val lastCol = min(cols, firstCol + (paint.width / metrics.width).toInt() + 1)
        val window = ColumnWindow(firstCol, lastCol, cols, rows.historyRows)

        ColumnIndicator(
            window = window,
            onOpen = { view.sheetOpen = true },
            modifier = Modifier
                .align(Alignment.BottomStart)
                .padding(bottom = with(density) { chromeBottom.toDp() }),
        )

        view.selection?.let { selection ->
            SelectionLayer(
                selection = selection,
                originX = geometry.originX,
                originY = geometry.originY,
                cellWidth = metrics.width,
                cellHeight = metrics.height,
                accent = tokens.color.accent,
                onAnchor = { view.selection = selection.copy(anchor = probe.cellAt(it)) },
                onHead = { view.selection = selection.copy(head = probe.cellAt(it)) },
                onCopy = {
                    clipboard.setText(AnnotatedString(logical.copy(selection)))
                    view.selection = null
                },
                onBlock = {
                    view.blockSelect = !view.blockSelect
                    view.selection = selection.copy(block = view.blockSelect)
                },
                block = view.blockSelect,
            )
        }

        view.target?.let { target ->
            TargetStrip(
                target = target,
                onAct = {
                    when (target.kind) {
                        TargetKind.Path -> clipboard.setText(AnnotatedString(target.text))
                        else -> uris.openUri(target.text)
                    }
                    view.target = null
                },
                onDismiss = { view.target = null },
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .padding(bottom = with(density) { (chromeBottom + INDICATOR_HEIGHT_DP.dp.toPx()).toDp() }),
            )
        }

        session.confirm.held?.let { held ->
            ConfirmSheet(
                held = held,
                onRun = {
                    sink.confirmed(held.payload)
                    session.confirm.held = null
                },
                onEdit = { session.confirm.held = null },
                onMute = {
                    session.confirm.local = false
                    io.send(ClientMsg.SetPrefs(pane.id, mapOf("confirm" to "off")))
                    sink.confirmed(held.payload)
                    session.confirm.held = null
                },
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .padding(bottom = with(density) { session.keyRowHeight.toDp() }),
            )
        }

        if (view.sheetOpen) {
            ZoomSheet(
                presets = presets,
                zoom = view.displayZoom,
                window = window,
                totalRows = rows.total,
                visibleRows = (paint.contentHeight / metrics.height).toInt(),
                remembered = view.remembered,
                followCursor = view.followCursor,
                confirmRisky = guard.wanted(),
                onZoom = { view.setZoom(it, presets) },
                onRemember = { view.remembered = it },
                onFollow = { view.followCursor = it },
                onConfirmRisky = { on ->
                    session.confirm.local = on
                    io.send(ClientMsg.SetPrefs(pane.id, mapOf("confirm" to if (on) "on" else "off")))
                },
                onDismiss = { view.sheetOpen = false },
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .padding(bottom = with(density) { session.keyRowHeight.toDp() }),
            )
        }
    }
}

private fun formatZoom(value: Float): String {
    val hundredths = (value * 100f + 0.5f).toInt()
    return "${hundredths / 100}.${(hundredths % 100).toString().padStart(2, '0')}"
}
