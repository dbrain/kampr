package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.absolutePadding
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
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntOffset
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
import dev.kampr.shared.model.othersWatching
import dev.kampr.shared.model.watchersPhrase
import dev.kampr.shared.platform.LocalReduceMotion
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.terminalPalette
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.breakpointOf
import dev.kampr.shared.ui.gestureAction
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
import dev.kampr.terminal.review.ReviewSurface
import dev.kampr.terminal.review.historyEdgeLabel
import dev.kampr.terminal.review.historyEdgeSpoken
import dev.kampr.terminal.review.historyWarning
import androidx.compose.runtime.snapshotFlow
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

private const val CURSOR_BLINK_MS = 530L

// What the strip is assumed to take for the one frame before it has been measured. The measured
// height replaces it immediately; nothing but the first paint ever depends on this number.
private const val INDICATOR_FLOOR_DP = 36f
private const val DECAY = 0.94f
private const val PREFS_DEBOUNCE_MS = 400L
private const val FONT_SETTLE_FRAMES = 120

// ADR 0010. The cursor line is the unit of speech, and a live region wired straight to `revision`
// would speak over itself at frame rate — so it settles first and speaks the line once.
private const val SPEECH_SETTLE_MS = 450L

// Mirrors the header PaneScreen floats over this surface and the answer strip it shows while a
// prompt is outstanding. Chrome insets the scrollable content; it never insets the paint.
private const val PENDING_BAR_DP = 52f

// The review strip is chrome like any other: without insetting for it the row the reader is
// being read is the row sitting behind the controls that read it.
private const val REVIEW_BAR_DP = 52f

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
    val probe = session.grid
    val review = session.review
    // LocalClipboard's ClipEntry is constructed from a platform-native object in CMP 1.11.1,
    // so the deprecated ClipboardManager is still the only clipboard reachable from common code.
    @Suppress("DEPRECATION")
    val clipboard = LocalClipboardManager.current
    val uris = LocalUriHandler.current
    val view = session.view
    val ground = palette.background(pane.styles[0])

    val stillness = LocalReduceMotion.current
    var cursorOn by remember { mutableStateOf(true) }
    LaunchedEffect(pane, stillness) {
        if (stillness) {
            cursorOn = true
            return@LaunchedEffect
        }
        while (true) {
            delay(CURSOR_BLINK_MS)
            cursorOn = !cursorOn
        }
    }

    fun reviewSurface() = ReviewSurface(rows, logical, rows.historyRows + pane.cursor.row)

    // ADR 0010: what a screen reader is given instead of 74x30 cells. Review turns it off — the
    // reader walking the grid and the pane reading its caret line would speak over each other, and
    // only one of the two was asked for.
    var spokenLine by remember(pane.id) { mutableStateOf("") }
    LaunchedEffect(pane, logical, rows, review.active) {
        if (review.active) return@LaunchedEffect
        snapshotFlow { pane.revision to pane.cursor }.collectLatest {
            delay(SPEECH_SETTLE_MS)
            val line = logical.lineAt(rows.historyRows + pane.cursor.row).first.trim()
            spokenLine = if (line.isEmpty()) "blank line" else line
        }
    }

    // Resolving the anchor against the surface as it is now is how a reader learns that the row
    // they were on has been repainted, or discarded outright. It never moves them.
    LaunchedEffect(pane, logical, rows) {
        snapshotFlow { pane.revision }.collect { review.sync(reviewSurface()) }
    }

    BoxWithConstraints(modifier.fillMaxSize().background(ground)) {
        val density = LocalDensity.current
        val breakpoint = breakpointOf(maxWidth, maxHeight)
        // The key row already stands off the gesture handle, so its measured height is normally
        // the taller of the two. The floor is for the layouts that have no key row at all — the
        // grid is still allowed under the handle, the controls floating over it are not.
        val safe = LocalSafeArea.current
        val chromeBottom =
            max(session.keyRowHeight, with(density) { safe.bottom.toPx() }) + pendingInsetPx(pane, density)
        // A cell in a mosaic is landscape-shaped but wears a much shorter header, and guessing
        // from its own size is what would leave blank rows under the last line.
        val chromeTop = LocalPaneChrome.current?.top ?: headerInsetDp(breakpoint).dp
        // The strip measures itself. It carries the review bar when review is on, a pill sized by
        // the touch rule and text sized by the type scale, and every one of those moves it.
        val strip = if (session.indicatorHeight > 0f) {
            session.indicatorHeight
        } else {
            with(density) { (INDICATOR_FLOOR_DP + if (review.active) REVIEW_BAR_DP else 0f).dp.toPx() }
        }
        val paint = PaintRect(
            width = with(density) { maxWidth.toPx() },
            height = with(density) { maxHeight.toPx() },
            insetTop = with(density) { chromeTop.toPx() },
            insetBottom = chromeBottom + strip,
        )

        var fontEpoch by remember(cache) { mutableIntStateOf(0) }
        val base = remember(cache, fontEpoch) { cache.metrics(BASE_CELL_SP.sp) }
        val cols = pane.cells.cols
        val presets = remember(paint.width, cols, base) { zoomPresets(paint.width, cols, base.width) }
        val stored = io.prefs(pane.id).zoom

        // Nothing is adopted before the first grid.reset: the placeholder buffer is 80x24 and a
        // zoom taken against it sticks for the life of the pane.
        // insetTop arrives a frame late — the chrome above this surface has to be laid out before
        // it can be measured — and a scroll clamped against the guess stays clamped there.
        LaunchedEffect(pane.id, pane.painted, cols, rows.historyRows > 0, paint.width, paint.insetTop, stored) {
            if (!pane.painted || view.chosen || view.scrolled) return@LaunchedEffect
            val fill = defaultZoom(
                paint, cols, rows.liveRows, rows.historyRows, base.width, base.height,
            )
            if (stored != null) view.setZoom(stored, presets) else view.adoptDefault(fill)
        }
        val zoom = if (view.zoom > 0f) view.zoom else 1f
        val metrics = remember(cache, zoom, fontEpoch) { cache.metrics((BASE_CELL_SP * zoom).sp) }
        LaunchedEffect(cache, zoom) {
            repeat(FONT_SETTLE_FRAMES) {
                withFrameNanos { }
                if (cache.reprobe((BASE_CELL_SP * zoom).sp)) fontEpoch++
            }
        }

        var carriedTotal by remember(pane.id) { mutableIntStateOf(rows.total) }
        if (rows.total != carriedTotal) {
            view.carryHistory(rows.total - carriedTotal, metrics.height)
            carriedTotal = rows.total
        }

        // Where the surface is allowed to rest. Re-derived every frame rather than placed once,
        // because the two things that move it — the caret, and the height of the rectangle the
        // keyboard leaves behind — both move long after the first paint. Placing it once is why
        // raising the keyboard took the prompt off the top and left the operator typing blind.
        val floor = caretFloor(paint, rows.total, rows.historyRows + pane.cursor.row, metrics.height)
        view.minScroll = floor
        // Review positions the viewport from the row being read and reaches rows this floor holds
        // off, so it is the one surface the floor does not govern.
        LaunchedEffect(floor, review.active, view.following) {
            if (review.active) return@LaunchedEffect
            view.scrollY = if (view.following) floor else max(view.scrollY, floor)
        }

        val edgeLabel = historyEdgeLabel(reviewSurface())
        val edgePad = if (edgeLabel == null) 0f else with(density) { HISTORY_EDGE_DP.toPx() }
        val geometry = terminalGeometry(
            paint, cols, rows.total, metrics.width, metrics.height, view.panX, view.scrollY, edgePad,
        )
        view.minPanX = geometry.minPanX
        view.maxScroll = geometry.maxScroll

        // The review cursor is the reader's; the viewport follows it rather than the other way
        // round, so what is spoken and what is painted are the same row.
        LaunchedEffect(review.active, review.row, review.reads, geometry.originY) {
            if (!review.active) return@LaunchedEffect
            val top = geometry.originY + review.row * metrics.height
            // Row 0 reveals the mark that says where the record stops, rather than stopping flush
            // against the header with it still above the fold.
            val wanted = paint.insetTop + if (review.row == 0) edgePad else 0f
            val shift = when {
                top < wanted -> wanted - top
                top + metrics.height > paint.contentBottom -> paint.contentBottom - top - metrics.height
                else -> 0f
            }
            if (shift != 0f) view.scrollY = (view.scrollY + shift).coerceIn(0f, view.maxScroll)
        }

        LaunchedEffect(pane.cursor, view.followCursor, geometry.pinned) {
            if (view.followCursor && geometry.pinned && !view.pinching) {
                view.panX = followCursorPan(
                    view.panX, geometry.minPanX, pane.cursor.col, metrics.width, paint.width,
                )
            }
        }

        LaunchedEffect(view.flings, stillness) {
            if (stillness) {
                view.velocityX = 0f
                view.velocityY = 0f
                return@LaunchedEffect
            }
            while (abs(view.velocityX) > 1f || abs(view.velocityY) > 1f) {
                withFrameNanos { }
                view.scrollBy(view.velocityX / 60f, view.velocityY / 60f)
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

        val visibleRows = (paint.contentHeight / metrics.height).toInt().coerceAtLeast(1)
        val info = io.info(pane.id)
        val transcript = info?.hasConversation == true
        val gridSummary = buildString {
            append("Terminal grid, $cols columns by ${rows.liveRows} rows")
            append(", cursor on row ${pane.cursor.row + 1}, column ${pane.cursor.col + 1}")
            if (rows.historyRows > 0) append(", ${rows.historyRows} rows of history above")
            historyWarning(reviewSurface())?.let { append(", $it") }
            if (pane.stale) append(", stale — frames have stopped arriving")
            if (io.readOnly) append(", read-only")
            // Read when the grid is reached rather than pushed: the pane's own notice is what
            // announces an arrival, and this is what answers the question afterwards.
            watchersPhrase(othersWatching(info))?.let { append(", $it") }
            append(
                if (transcript) {
                    ". A cell grid does not linearise; the Conversation view of this pane is " +
                        "ordinary text and is the surface to read it on."
                } else {
                    ". Only the line under the cursor is spoken."
                }
            )
        }

        Box(
            Modifier
                .fillMaxSize()
                .gestureAction(
                    label = gridSummary,
                    onClick = { session.openKeyboard() },
                    clickLabel = "Type into this pane",
                    actions = buildList {
                        if (review.active) {
                            add("Leave review" to { review.leave() })
                        } else {
                            add("Review this pane row by row" to { session.closeKeyboard(); review.enter(reviewSurface()) })
                        }
                        if (transcript) add("Open the Conversation view" to { io.show(PaneView.Conversation) })
                    },
                )
                // A sheet over this surface is modal, and the grid's detector consumes the
                // release — so leaving it live is what let a tap meant for the scrim raise the
                // keyboard instead of closing the sheet.
                .then(
                    if (view.sheetOpen || session.confirm.held != null) {
                        Modifier
                    } else {
                        Modifier.pointerInput(pane.id) {
                            terminalGestures(session, presets, paint, probe, ::tapped)
                        }
                    },
                ),
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
                        if (review.active) {
                            val top = geometry.originY + review.row * metrics.height
                            drawRect(
                                tokens.color.accentSoft,
                                topLeft = Offset(0f, top),
                                size = androidx.compose.ui.geometry.Size(size.width, metrics.height),
                            )
                        }
                        if (pane.stale) drawRect(tokens.color.bg.copy(alpha = 0.45f), size = size)
                    },
            )
        }

        // A one-dp strip carrying the cursor line: nothing to look at, and the only thing on this
        // surface a screen reader can follow as the pane writes.
        if (!review.active) {
            Box(
                Modifier
                    .align(Alignment.TopStart)
                    .fillMaxWidth()
                    .height(1.dp)
                    .announce(spokenLine),
            )
        }

        // Where the record stops, laid out at the top of the scrollable surface so it is seen by
        // scrolling to it rather than worn as a badge.
        if (edgeLabel != null) {
            HistoryEdgeMark(
                label = edgeLabel,
                spoken = historyEdgeSpoken(reviewSurface()),
                broken = historyWarning(reviewSurface()) != null,
                modifier = Modifier
                    .align(Alignment.TopStart)
                    .layout { measurable, constraints ->
                        val placeable = measurable.measure(constraints)
                        layout(placeable.width, placeable.height) {
                            placeable.place(IntOffset(0, (geometry.originY - edgePad).toInt()))
                        }
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

        Column(
            Modifier
                .align(Alignment.BottomStart)
                .fillMaxWidth()
                .absolutePadding(
                    left = safe.left,
                    right = safe.right,
                    bottom = with(density) { chromeBottom.toDp() },
                )
                // Inside the padding on purpose: the paint already insets for the chrome this
                // stands off, and measuring the padded node would count that chrome twice.
                .onSizeChanged { session.indicatorHeight = it.height.toFloat() },
        ) {
            if (review.active) {
                ReviewStrip(
                    review = review,
                    total = rows.total,
                    warning = historyWarning(reviewSurface()),
                    onMove = { review.step(reviewSurface(), it) },
                    onLeave = { review.leave() },
                )
            }
            ColumnIndicator(
                window = window,
                reviewing = review.active,
                onOpen = { view.sheetOpen = true },
                onReview = { session.closeKeyboard(); review.enter(reviewSurface()) },
            )
        }

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
                    .padding(bottom = with(density) { (chromeBottom + strip).toDp() }),
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
                visibleRows = visibleRows,
                historyNote = historyWarning(reviewSurface()),
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
