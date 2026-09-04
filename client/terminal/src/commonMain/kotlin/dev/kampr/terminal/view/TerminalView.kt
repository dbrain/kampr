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
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
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
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.PointerIcon
import androidx.compose.ui.input.pointer.isSecondaryPressed
import androidx.compose.ui.input.pointer.pointerHoverIcon
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
import dev.kampr.shared.platform.LocalClipboardText
import dev.kampr.shared.platform.LocalReduceMotion
import dev.kampr.shared.platform.filePickAvailable
import dev.kampr.shared.platform.PastedFiles
import dev.kampr.shared.platform.pickFile
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.terminalPalette
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.LocalMosaicCell
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.breakpointOf
import dev.kampr.shared.ui.gestureAction
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.MIN_PANE_COLS
import dev.kampr.shared.wire.MIN_PANE_ROWS
import dev.kampr.shared.wire.SizeMode
import dev.kampr.shared.wire.talks
import dev.kampr.terminal.PaneSession
import dev.kampr.terminal.file.Handover
import dev.kampr.terminal.file.handoverAfter
import dev.kampr.terminal.file.handoverName
import dev.kampr.terminal.file.handoverOf
import dev.kampr.terminal.guard.SubmitGuard
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.PaneChord
import dev.kampr.terminal.input.PaneTextInput
import dev.kampr.terminal.render.GridRenderer
import dev.kampr.terminal.render.ModeSelector
import dev.kampr.terminal.render.LogicalText
import dev.kampr.terminal.render.ResolvedStyles
import dev.kampr.terminal.render.GridPoint
import dev.kampr.terminal.render.Selection
import dev.kampr.terminal.render.Target
import dev.kampr.terminal.render.TargetKind
import dev.kampr.terminal.render.detectTarget
import dev.kampr.terminal.render.SurfaceRows
import dev.kampr.terminal.render.TextCache
import dev.kampr.terminal.review.ReviewSurface
import dev.kampr.terminal.review.historyEdgeLabel
import dev.kampr.terminal.review.historyEdgeSpoken
import dev.kampr.terminal.input.PaneScroll
import dev.kampr.terminal.input.paneScrollKeys
import dev.kampr.terminal.review.historyWarning
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
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

// How long the caret has to hold still before the band is moved to it. Long enough to cover a
// repaint's whole sweep — #380 measured those at several a second, and one is a single batch of
// writes — and short enough that a caret that really did move somewhere off screen is fetched
// back inside one repaint interval.
internal const val CARET_SETTLE_MS = 200L

// Mirrors the header PaneScreen floats over this surface and the answer strip it shows while a
// prompt is outstanding. Chrome insets the scrollable content; it never insets the paint.
private const val PENDING_BAR_DP = 52f

// The review strip is chrome like any other: without insetting for it the row the reader is
// being read is the row sitting behind the controls that read it.
private const val REVIEW_BAR_DP = 52f

// The wash over the cells a click hit, and the rule under them. Light enough that the glyphs it
// covers are still read through it, which a `selectionWash` sized for an unread block is not.
private const val TARGET_WASH = 0.22f
private const val TARGET_RULE = 2f

// The card is placed at the click, not under it: a pointer sitting on its top-left corner hides
// the first characters of the thing it is naming.
private const val CARD_NUDGE = 10f

private fun headerInsetDp(breakpoint: Breakpoint): Float = when (breakpoint) {
    Breakpoint.Desktop -> 56f
    Breakpoint.Landscape -> 44f
    Breakpoint.Portrait -> 108f
}

private fun pendingInsetPx(pane: PaneState, density: Density): Float =
    if (pane.pending != null) with(density) { PENDING_BAR_DP.dp.toPx() } else 0f

// How long the window has to hold still before its size is asked for.
//
// A drag is hundreds of sizes, and each claim is a `herdr terminal session control` child. The
// coroutine is cancelled by the next size, so a drag costs one claim at the end of it rather than
// one per frame — and the `insetTop` this is measured against arrives a frame late, which this
// also absorbs.
private const val MATCH_SETTLE_MS = 250L

// Holds the pane at this view's geometry for as long as this view is open, and lets go when it is
// not — a switch to the conversation, a pane closed, a window that stopped being desk-sized, the
// switch turned off. ADR 0013.
//
// **The release the operator cannot send is the node's**, not this: a closed laptop never reaches
// here. What this covers is the ordinary end of a view; `session.rs` covers the rest.
@Composable
private fun MatchTheView(
    paneId: String,
    io: PaneIo,
    on: Boolean,
    cols: Int,
    rows: Int,
) {
    // Whether the session actually took the pane. It may decline one already close enough to this
    // view to be worth a reflow, and everything below answers to *that* rather than to the switch:
    // a strip saying a pane is held, and a release for a hold nobody took, are both lies.
    //
    var claimed by remember(paneId) { mutableStateOf(false) }
    LaunchedEffect(paneId, on, cols, rows) {
        if (!on) return@LaunchedEffect
        delay(MATCH_SETTLE_MS)
        claimed = io.claimMatch(paneId, cols, rows)
    }
    // The status strip is what stops this being a shape change nobody was told about: it says the
    // pane is being held while it is, whether the operator ticked the switch or their screen size
    // did.
    // Snapshotted, because `onDispose` closes over the *state* and not over its value: read
    // straight, it sees `claimed` as it is when the effect is torn down, so the false-to-true flip
    // that follows a successful claim disposed the old effect and released a hold that had just
    // been taken. What this effect is holding is what it was set up with.
    DisposableEffect(paneId, claimed) {
        val holding = claimed
        io.holding(paneId, holding)
        onDispose {
            if (holding) {
                io.holding(paneId, false)
                io.releaseMatch(paneId)
            }
        }
    }
}

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
    val peek = session.peek
    // LocalClipboard's ClipEntry is constructed from a platform-native object in CMP 1.11.1,
    // so the deprecated ClipboardManager is still the only clipboard reachable from common code.
    @Suppress("DEPRECATION")
    val clipboard = LocalClipboardManager.current
    // Reading is the direction that deprecated clipboard cannot do: its wasm actual is a hard-coded
    // null, so a Paste wired to it would be a button that silently does nothing on the web.
    val readClipboard = LocalClipboardText.current
    val uris = LocalUriHandler.current
    val view = session.view
    val ground = palette.background(pane.styles[0])

    val scope = rememberCoroutineScope()
    // A node refuses a paste with an error naming this pane — too large, not base64, nowhere to
    // write — and that error is quiet everywhere else by design, so this is the only place on this
    // surface it can be said.
    LaunchedEffect(pane.refusal) { session.handover = handoverAfter(session.handover, pane.refusal) }
    // The same handover as the attach button's, reached by the gesture a desk actually uses. A
    // clipboard with no file on it never gets here, so ctrl+v of ordinary text still goes to the
    // pane the way it always has.
    PastedFiles(!io.readOnly) { picked ->
        session.handover = Handover.Going(handoverName(picked))
        session.handover = handoverOf(pane, io, picked)
    }

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
            val line = logical.lineAt(rows.historyRows + pane.cursor.row, pane.cursor.col).first.trim()
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
        LaunchedEffect(
            pane.id, pane.painted, cols, rows.historyRows > 0, paint.width, paint.insetTop, stored, breakpoint,
        ) {
            if (!pane.painted || view.chosen || view.scrolled) return@LaunchedEffect
            val fill = defaultZoom(
                paint, cols, rows.liveRows, rows.historyRows, base.width, base.height,
                ceiling = if (breakpoint == Breakpoint.Desktop) 1f else Float.MAX_VALUE,
            )
            if (stored != null) view.setZoom(stored, presets) else view.adoptDefault(fill)
        }
        // **The standing intent, and the only automatic claim in the product.** It is 0012's op
        // under a setting the operator can see and turn off, not a second way to resize —
        // ADR 0013. The gate is the viewport *this surface* measured for itself rather than the
        // window's, so a split half and a phone in landscape fall out on the same test a phone
        // does; a mosaic cell can be desk-sized and is named separately.
        val (viewCols, viewRows) = viewGrid(paint, base.width, base.height)
        val roomToMatch = viewCols >= MIN_PANE_COLS && viewRows >= MIN_PANE_ROWS
        val matchAsked = view.matchView ?: io.prefs(pane.id).matchView
        // A fleet pane is a pty this node forked for a job of its own, with its geometry fixed
        // when the run started and no operator desk to trample — rule 3's other half. It is not a
        // herdr pane either, so `pane.size` refuses one outright.
        val ownPane = io.info(pane.id)?.fleet != null
        val matching = !io.readOnly && !ownPane && roomToMatch && !LocalMosaicCell.current &&
            (matchAsked ?: (breakpoint == Breakpoint.Desktop))
        MatchTheView(pane.id, io, matching, viewCols, viewRows)

        val zoom = if (view.zoom > 0f) view.zoom else 1f
        val metrics = remember(cache, zoom, fontEpoch) { cache.metrics((BASE_CELL_SP * zoom).sp) }
        LaunchedEffect(cache, zoom) {
            repeat(FONT_SETTLE_FRAMES) {
                withFrameNanos { }
                if (cache.reprobe((BASE_CELL_SP * zoom).sp)) fontEpoch++
            }
        }

        var carriedTotal by remember(pane.id) { mutableIntStateOf(rows.total) }
        // A ring that was thrown away and started again is re-delivery, not output: atuin's history
        // search takes the alternate screen (`?1049h`, measured — probe #475) and the node stops
        // vouching for the shell's ring for as long as it is up, so pressing up and pressing escape
        // is the whole ring leaving and arriving inside a second. Carrying that moved a parked
        // reader by three hundred rows and the discard's own clamp at zero meant the two moves
        // could never cancel — the operator's *"the terminal scroll up and I need to manually
        // scroll down"*. The surface is re-based on what came back instead.
        var carriedRestarts by remember(pane.id) { mutableIntStateOf(rows.restarts) }
        if (rows.restarts != carriedRestarts) {
            carriedRestarts = rows.restarts
            carriedTotal = rows.total
        }
        if (rows.total != carriedTotal) {
            view.carryHistory(rows.total - carriedTotal, metrics.height)
            carriedTotal = rows.total
        }

        // Where the caret has *stopped*, counted from the bottom of the surface, which is the only
        // coordinate the band is a function of and the only one history arriving leaves alone.
        //
        // The live caret cannot be used. The band translates with it one pixel per pixel, so an
        // excursion wider than the band drags the viewport in both directions once per frame — and
        // a full-screen redraw is exactly that excursion the moment the grid is taller than the
        // rectangle it is shown in. #380 fixed the shallow case; the operator on a zoomed-in desk
        // watched the deep one: "sometimes it flashes and sort of scrolls up then down". A repaint
        // walks the caret home and back inside one batch of writes and rests where it stops, so
        // waiting for it to hold still tells the sweep and the destination apart with nothing else
        // to go on.
        //
        // Re-seeded rather than settled on the pane's first real grid: before `painted` the buffer
        // is the 80x24 placeholder, and waiting out an interval on that would open every pane on
        // the wrong band.
        val below = rows.liveRows - pane.cursor.row
        var settledBelow by remember(pane.id, pane.painted) { mutableIntStateOf(below) }
        // Keyed on the reading rather than collected from a `snapshotFlow`, because `CellBuffer`
        // is not snapshot state: a resize changes how many rows sit below the caret without
        // moving the caret, and only a composition sees that. Re-keying is the cancellation —
        // a caret that has not stopped never reaches the assignment.
        LaunchedEffect(pane.id, below) {
            delay(CARET_SETTLE_MS)
            settledBelow = below
        }

        // And where the *record* stops, which on a herdr pane is a different row: the desk chose
        // the height, the shell filled as much of it as it filled, and the rest is blank tail.
        //
        // Settled for the same reason the caret is, and on the same clock. A redraw that blanks a
        // block and rewrites it takes the content end to the top of that block and back inside one
        // batch of writes, and a floor that followed it would drag a following viewport up and
        // straight back down — which is #428's flash arriving by the other of the two floors.
        // `collectLatest` is the whole of that rule: a reading that does not survive the interval
        // is never taken.
        //
        // Off the cells and therefore out of an effect, never out of the composition. The walk is
        // cheap but `pane.revision` is not a thing this composable reads today, and reading one to
        // key a walk on would put every line of this function behind every frame the pane paints.
        // What it costs instead is one walk per quiet interval — and none at all while a pane is
        // painting, because a flow that is restarted never reaches its own body.
        var settledContent by remember(pane.id, pane.painted) {
            mutableIntStateOf(rows.contentBelow(pane.cursor.row))
        }
        LaunchedEffect(pane, rows) {
            snapshotFlow { pane.revision }.collectLatest {
                delay(CARET_SETTLE_MS)
                settledContent = rows.contentBelow(pane.cursor.row)
            }
        }

        // Where the surface is allowed to rest. Re-derived every frame rather than placed once,
        // because the two things that move it — the caret, and the height of the rectangle the
        // keyboard leaves behind — both move long after the first paint. Placing it once is why
        // raising the keyboard took the prompt off the top and left the operator typing blind.
        //
        // The caret is content wherever it is, so the content can never end above it — the two
        // readings settle on their own clocks and only the smaller of the two is a distance the
        // surface may be held off the bottom by.
        val contentBelow = min(settledContent, settledBelow)
        val band = caretBand(
            paint, rows.total, rows.total - settledBelow, rows.total - contentBelow, metrics.height,
        )
        view.band = band
        view.contentFloor = contentFloor(paint, rows.total, rows.total - contentBelow, metrics.height)
        var placedCell by remember(pane.id) { mutableFloatStateOf(0f) }
        if (placedCell != metrics.height) {
            placedCell = metrics.height
            view.placeOnFloor(band.floor)
        }
        // A reader who is following stays exactly where they are for as long as the caret is on
        // screen, and is moved the least the band allows when it is not.
        //
        // A reader who has parked is not moved at all. The floor used to be held under them as
        // well — `max(scrollY, floor)` — and that is the other half of the report: the bottom of a
        // tall grid sits below the floor, so every hand that reached it was thrown back to the
        // caret by the next frame. Typing is what puts a parked reader back on the live edge, and
        // it is the only thing that does.
        //
        // Review positions the viewport from the row being read and reaches rows the band holds
        // off, so it is the one surface the band does not govern.
        LaunchedEffect(band, review.active, view.following) {
            if (review.active) return@LaunchedEffect
            if (view.following) view.scrollY = view.scrollY.coerceIn(band.floor, band.ceiling)
        }

        // Every byte this client sends the pane, and the two things that owe it an answer. The
        // viewport goes back to following, because typing is a request to be shown what you typed
        // and a hand that parked below the live edge has no other way back. And the handover line
        // stands down: "<name> is on the agent's machine, and its path is typed in" is a fact about
        // a composer that stops being true the moment anything else is typed into it.
        LaunchedEffect(sink, session) {
            snapshotFlow { sink.sends }.drop(1).collect {
                view.followAgain()
                if (session.handover is Handover.Sent) session.handover = Handover.Idle
            }
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

        // Asked of `following`, not of scroll zero. Scroll zero is the bottom of the surface and
        // stopped being where a follower rests at #175; the caret band moved it again. A pane at a
        // zoom the operator picked overflows both axes, and gating this on the bottom left the
        // caret two screen widths off the right edge with no frame able to bring it back (#380).
        LaunchedEffect(pane.cursor, view.followCursor, view.following) {
            if (view.followCursor && view.following && !view.pinching) {
                view.chaseCursor(
                    followCursorPan(
                        view.panX, geometry.minPanX, pane.cursor.col, metrics.width, paint.width,
                    )
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

        // One copy and one paste on this surface, reached three ways: the selection pill, the
        // keyboard chord, and the grid's own context menu. A second implementation of either is
        // how two of them come to disagree about bracketing, about the read-only rule, or about
        // whether the pill goes away afterwards.
        fun copySelection() {
            val selection = view.selection ?: return
            clipboard.setText(AnnotatedString(logical.copy(selection)))
            view.selection = null
        }

        // Bracketed by `InputSink.paste`, so a multi-line paste reaches a shell as one block rather
        // than executing line by line (#9), and inspected by the guard on the way past like
        // anything else that arrives carrying its own Enter. The selection goes first: the read is
        // what raises Android's clipboard notice, and leaving the pill up under it reads as a press
        // that did nothing.
        //
        // Null on a read-only device rather than present-and-refusing, which is `ManageLayer`'s
        // rule for everything a write can reach — the pill and the menu both read it as "offer
        // nothing", and the chord as "do nothing".
        val pasteIntoPane: (() -> Unit)? = if (io.readOnly) {
            null
        } else {
            {
                view.selection = null
                scope.launch {
                    val text = readClipboard()
                    if (text.isNullOrEmpty()) {
                        session.handover = Handover.Refused("Nothing on the clipboard to paste.")
                    } else {
                        sink.paste(text)
                    }
                }
            }
        }

        fun tapped(position: Offset) {
            view.menuAt = null
            if (view.selection != null) {
                view.selection = null
                view.aimOff()
                return
            }
            val cell = probe.cellAt(position)
            val declared = pane.links.getOrNull(logical.linkAt(cell.row, cell.col))
            if (declared != null) {
                view.aim(Target(declared, TargetKind.Link), logical.linkSpan(cell.row, cell.col), position)
                return
            }
            val (line, offset) = logical.lineAt(cell.row, cell.col)
            val found = detectTarget(line, offset)
            // The route is gated on a device that may send input, and the whole argument for a
            // client-minted file id is that such a device can already `cat` the file. A device
            // that may not type is offered the string instead of the bytes.
            val offered = if (found?.kind == TargetKind.File && io.readOnly) {
                found.copy(kind = TargetKind.Path)
            } else {
                found
            }
            view.aim(offered, offered?.let { logical.spanOf(cell.row, it.range) }, position)
            if (found == null) session.openKeyboard()
        }

        val visibleRows = (paint.contentHeight / metrics.height).toInt().coerceAtLeast(1)
        val info = io.info(pane.id)
        val transcript = info.talks
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

        // A program that holds the alternate screen keeps no ring behind it (#387), so a gesture
        // over one moved nothing at all — the report was "some terminals I can't scroll up on,
        // unclear why". The scroll goes to the program instead, which is what every terminal does
        // and what herdr already does at the desk: the wheel by the notch, a finger by the row,
        // once the surface underneath is spent. `paneScrollKeys` decides whether anything may be
        // sent at all and in what dialect; a read-only viewer sends nothing whatever it says,
        // because these are pty bytes.
        val scrollToPane = when {
            io.readOnly || rows.historyRows > 0 -> null
            else -> paneScrollKeys(info?.agent, info?.cmd)?.let { keys ->
                PaneScroll(keys) { report -> io.send(ClientMsg.InputText(pane.id, report)) }
            }
        }

        Box(
            Modifier
                .fillMaxSize()
                // The grid paints into a Canvas, so nothing under the pointer asks for a cursor
                // and the whole terminal hovered as a plain arrow. It is a surface a mouse selects
                // text on — `mouseGesture` drags a selection out of it — so it wears the I-beam,
                // selection or no selection, menu or no menu. `pressable` is for controls and
                // would be wrong here.
                //
                // On this box rather than on the layer inside it, which is the same rectangle and
                // is this box's only child: a hover icon is only ever readable off the layout node
                // that carries a node's semantics, and the layer carries none — so put on the
                // layer, the one cursor on this screen that is not a control's was the one cursor
                // no test could see.
                .pointerHoverIcon(PointerIcon.Text)
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
                // The wheel stays live under a sheet. It consumes nothing but a scroll, so it
                // cannot steal the tap the scrim needs — and ctrl+wheel while the zoom sheet is
                // open is the sheet's own readout moving, which is the point of having it there.
                .pointerInput(pane.id, scrollToPane != null) { terminalWheel(view, probe, presets, scrollToPane) }
                // A right-click over the text, which is the one surface on this screen that had
                // no menu at all. On the **Main** pass and only on an unconsumed press, which is
                // what keeps it out of a mosaic: `Modifier.paneActions` on a cell runs on the
                // Initial pass and consumes, so inside a mosaic the cell's own sheet still wins
                // and the grid never sees the gesture.
                //
                // The raw event rather than `awaitFirstDown`, for `paneActions`' reason: on skiko
                // a first down refers to the primary button alone and never returns for a press
                // carrying the second one.
                .pointerInput(pane.id) {
                    awaitEachGesture {
                        val event = awaitPointerEvent()
                        if (event.type != PointerEventType.Press) return@awaitEachGesture
                        if (event.changes.any { it.isConsumed }) return@awaitEachGesture
                        if (!event.buttons.isSecondaryPressed) return@awaitEachGesture
                        event.changes.forEach { it.consume() }
                        view.aimOff()
                        view.menuAt = event.changes.first().position
                    }
                }
                // The touch detector does not. A sheet over this surface is modal, and the grid's
                // detector consumes the release — so leaving *it* live is what let a tap meant for
                // the scrim raise the keyboard instead of closing the sheet.
                .then(
                    if (view.sheetOpen || view.menuAt != null || session.confirm.held != null || peek.path != null) {
                        Modifier
                    } else {
                        Modifier.pointerInput(pane.id, scrollToPane != null) {
                            terminalGestures(session, presets, paint, probe, scrollToPane, ::tapped)
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
                        // What was hit, marked where it is. Without it the affordance names a path
                        // and the screen holds forty of them. A wash the text still reads through
                        // and a rule under it: `accentSoft` is opaque in half the themes and this
                        // is drawn over the glyphs, not behind them.
                        view.targetSpan?.let { span ->
                            for (row in span.start.row..span.end.row) {
                                val cells = span.span(row, cols) ?: continue
                                val left = geometry.originX + cells.first * metrics.width
                                val top = geometry.originY + row * metrics.height
                                val wide = (cells.last - cells.first + 1) * metrics.width
                                drawRect(
                                    tokens.color.accent.copy(alpha = TARGET_WASH),
                                    topLeft = Offset(left, top),
                                    size = androidx.compose.ui.geometry.Size(wide, metrics.height),
                                )
                                drawRect(
                                    tokens.color.accent,
                                    topLeft = Offset(left, top + metrics.height - TARGET_RULE),
                                    size = androidx.compose.ui.geometry.Size(wide, TARGET_RULE),
                                )
                            }
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

        // Stood down while a sheet is up, and the web is why. The wasm actual reclaims DOM focus
        // every animation frame and stands down only for an `INPUT`, a `TEXTAREA` or something
        // contenteditable — and Compose renders into a `<canvas>`, which is none of those. So the
        // offscreen div took the focus straight back off the sheet each frame and `preventDefault`ed
        // Escape and every ctrl chord into the shell, and no key ever reached the sheet at all.
        // `Modifier.modal`'s Escape-to-dismiss could not work in a browser while this was ungated.
        PaneTextInput(
            session = session,
            sink = sink,
            enabled = !io.readOnly && !view.sheetOpen && session.confirm.held == null && peek.path == null,
            onChord = { chord ->
                when (chord) {
                    PaneChord.Copy -> copySelection()
                    PaneChord.Paste -> pasteIntoPane?.invoke()
                }
            },
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
            HandoverLine(session.handover, info?.agent)
            ColumnIndicator(
                window = window,
                reviewing = review.active,
                onOpen = { view.sheetOpen = true },
                onReview = { session.closeKeyboard(); review.enter(reviewSurface()) },
                attachTo = info?.agent,
                onAttach = if (io.readOnly || !filePickAvailable) {
                    null
                } else {
                    {
                        scope.launch {
                            val picked = pickFile() ?: return@launch
                            session.handover = Handover.Going(handoverName(picked))
                            session.handover = handoverOf(pane, io, picked)
                        }
                    }
                },
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
                onCopy = { copySelection() },
                onPaste = pasteIntoPane,
                onBlock = {
                    view.blockSelect = !view.blockSelect
                    view.selection = selection.copy(block = view.blockSelect)
                },
                block = view.blockSelect,
            )
        }

        view.menuAt?.let { at ->
            GridMenu(
                at = at,
                onCopy = if (view.selection == null) null else ({ copySelection() }),
                onPaste = pasteIntoPane,
                onSelectAll = {
                    view.selection = Selection(
                        GridPoint(0, 0),
                        GridPoint((rows.total - 1).coerceAtLeast(0), (cols - 1).coerceAtLeast(0)),
                        view.blockSelect,
                    )
                },
                onDismiss = { view.menuAt = null },
            )
        }

        view.target?.let { target ->
            val act = {
                when (target.kind) {
                    TargetKind.Path -> clipboard.setText(AnnotatedString(target.text))
                    TargetKind.File -> scope.launch { peek.open(io, pane.id, target.text) }
                    else -> uris.openUri(target.text)
                }
                view.aimOff()
            }
            val anchor = view.targetAt
            // By form factor, not by platform. A desk clicks a path at the top of a nine-hundred
            // pixel pane and the bottom of the screen is nowhere near it; a phone has no room to
            // put a card beside the tap without putting it under the thumb that made it, and its
            // bottom edge is a finger's travel away rather than a mouse's. `Breakpoint.Desktop` is
            // already the line between the two — a phone in landscape is not one of them.
            if (breakpoint == Breakpoint.Desktop && anchor != null) {
                TargetCard(
                    target = target,
                    at = Offset(
                        anchor.x - CARD_NUDGE,
                        max(anchor.y + metrics.height, with(density) { chromeTop.toPx() }),
                    ),
                    onAct = act,
                    onDismiss = { view.aimOff() },
                )
            } else {
                TargetStrip(
                    target = target,
                    onAct = act,
                    onDismiss = { view.aimOff() },
                    modifier = Modifier
                        .align(Alignment.BottomStart)
                        .padding(bottom = with(density) { (chromeBottom + strip).toDp() }),
                )
            }
        }

        peek.path?.let { at ->
            FileSheet(
                path = at,
                state = peek.state,
                onClose = peek::close,
                onCopy = { clipboard.setText(AnnotatedString(it)) },
                // The pane header floats over this surface rather than sitting above it, so a
                // sheet that fills the surface and puts its controls at its own top edge puts
                // them underneath the header. That is where the Close button has been: painted,
                // reachable by tab, and covered by an opaque bar — which reads as no close button
                // at all, and left Escape as the only way out.
                chromeTop = chromeTop,
                chromeBottom = with(density) { chromeBottom.toDp() },
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
                // Hidden on a read-only device rather than disabled, which is `ManageLayer`'s rule
                // for everything `manage` can reach.
                sizing = if (io.readOnly) {
                    null
                } else {
                    PaneSizing(
                        cols = cols,
                        rows = rows.liveRows,
                        viewCols = viewCols,
                        viewRows = viewRows,
                        held = view.sizeHeld,
                        matching = matching,
                        canMatch = roomToMatch && !ownPane,
                    )
                },
                onResize = { c, r ->
                    io.send(
                        ClientMsg.Manage(
                            ManageOp.PaneSize(
                                at = pane.id,
                                cols = c,
                                rows = r,
                                mode = if (view.sizeHeld) SizeMode.Hold else SizeMode.Once,
                            ),
                        ),
                    )
                },
                onMatchView = { on ->
                    view.matchView = on
                    io.send(ClientMsg.SetPrefs(pane.id, mapOf("match" to if (on) "on" else "off")))
                    // Claiming stays `MatchTheView`'s: it is the one place that knows the size.
                    // Letting go does not, because the two releases are different events — this
                    // one is the operator answering about this pane and is owed it back now,
                    // where a view ending is given the linger a pane switch needs. The session
                    // lets go of nothing it is not holding, so an untick before any claim is
                    // silent rather than a release of somebody else's hold.
                    if (!on) io.releaseMatch(pane.id, linger = false)
                },
                onHoldSize = { on ->
                    view.sizeHeld = on
                    io.holding(pane.id, on)
                    // Ticking it off is the release. Ticking it on claims nothing by itself — the
                    // next resize is what takes the PTY, so the toggle is a choice about how the
                    // next one behaves rather than an action of its own.
                    if (!on) io.send(ClientMsg.Manage(ManageOp.PaneSize(pane.id, mode = SizeMode.Release)))
                },
                onDismiss = {
                    view.sheetOpen = false
                    // A hold outlives the panel only by mistake. The node releases at its own
                    // deadline regardless, but that is the backstop and this is the ordinary path.
                    if (view.sizeHeld) {
                        view.sizeHeld = false
                        io.holding(pane.id, false)
                        io.send(ClientMsg.Manage(ManageOp.PaneSize(pane.id, mode = SizeMode.Release)))
                    }
                },
                // The same two numbers the grid is painted between, handed to the sheet as its
                // box. Standing it off the key row alone left it free to grow up under the pane
                // header, which is where a third of it went; and `chromeBottom` rather than the
                // key row is what also holds it off a pending bar and off the gesture handle on a
                // layout that has no key row.
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .absolutePadding(
                        left = safe.left,
                        right = safe.right,
                        top = chromeTop,
                        bottom = with(density) { chromeBottom.toDp() },
                    ),
            )
        }
    }
}

private fun formatZoom(value: Float): String {
    val hundredths = (value * 100f + 0.5f).toInt()
    return "${hundredths / 100}.${(hundredths % 100).toString().padStart(2, '0')}"
}
