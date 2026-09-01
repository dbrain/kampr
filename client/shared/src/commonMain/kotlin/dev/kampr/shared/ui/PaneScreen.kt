package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.absolutePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.PaneGone
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.model.watchersTag
import dev.kampr.shared.platform.keyRowNeeded
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.talks

// What is actually rendered, against what was asked for. A remembered preference, a deep link and
// the desktop's own Split default all outlive the transcript that justified them, and the pane they
// name is still live — so the terminal is where that lands, and the request is kept rather than
// rewritten, which is what brings the operator's choice back when the transcript returns.
private fun viewOn(info: PaneInfo?, view: PaneView): PaneView =
    if (info.talks) view else PaneView.Terminal

// The room a conversation needs at its foot to be worth opening: one line of reply box, its
// padding, and the send button beside it — 70 dp, which `aReplyBoxIsNoTallerThanTheRoomReservedForIt`
// holds the composer to.
//
// **The pane header floats over the conversation and is paid for as a top padding**, so on a short
// window the two compete for the same pixels and the header, being measured first, wins all of
// them. Rotated with the keys up that left the reply box 0 dp tall (#319) — a pane you cannot type
// into, which is the defect this whole surface exists to prevent. A header you cannot read is worth
// less than a box you can type into, so the header is what yields.
val REPLY_ROOM: Dp = 70.dp

// Why this pane will never paint, when there is nothing on its surface to read instead. The
// node's own refusal about this pane stands in where the herd entry says nothing: a failure the
// global strip stayed quiet about — because the operator was on another pane when it arrived — is
// still the answer to why they are looking at an empty one now.
private fun streamFault(pane: PaneState, info: PaneInfo?): String? =
    (info?.detail ?: pane.refusal)?.takeUnless { pane.painted }

// The zoom sheet belongs to the terminal surface, so the control that opens it has nothing to open
// on a transcript. Withdrawing it outright is what the report is about: every one of these headers
// has something elastic in it, and a row that loses an item hands the width to whatever stretches.
// The portrait switch grew by the button's whole width; on a 360 dp phone and a 740 dp landscape
// the row rewrapped and the switch moved outright. Either way the segment under the thumb is not
// the one that gets tapped.
//
// So the control is measured whichever view is up, and placed only when it leads somewhere: the
// slot keeps its width and nothing beside it reflows. An unplaced child is neither painted nor
// hit — but its semantics outlive being unplaced, so those are cleared too. A reader offered a
// button that opens nothing is the inert affordance this header had taken out of it once already.
// Measured rather than given a constant because the button's width is its label's, and the label
// is the zoom: "fit" and "12.0×" are not the same slot.
@Composable
private fun ZoomSlot(pane: PaneState, surfaces: PaneSurfaces, shown: PaneView) {
    val opens = shown != PaneView.Conversation
    Layout(
        { surfaces.Zoom(pane, Modifier) },
        if (opens) Modifier else Modifier.clearAndSetSemantics {},
    ) { measurables, constraints ->
        val slot = measurables.map { it.measure(constraints) }
        layout(slot.maxOfOrNull { it.width } ?: 0, slot.maxOfOrNull { it.height } ?: 0) {
            if (opens) slot.forEach { it.place(0, 0) }
        }
    }
}

private fun statusLabel(status: AgentStatus): String? = when (status) {
    AgentStatus.Blocked, AgentStatus.Working, AgentStatus.Done -> statusWord(status)
    else -> null
}

@Composable
private fun StatusChip(info: PaneInfo?) {
    val tokens = Kampr.tokens
    val status = info?.let(::statusOf) ?: AgentStatus.Unknown
    val label = statusLabel(status) ?: return
    val tone = statusColor(status)
    Row(
        Modifier.announce("This agent is ${label.lowercase()}", urgent = status == AgentStatus.Blocked),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        StatusMark(status, 7.dp)
        StatusBadge(label, tone, if (status == AgentStatus.Blocked) tokens.color.blockedBg else tokens.color.surface)
    }
}

// The name a pane had, held past the herd entry that carried it. A pane that leaves the herd takes
// its title and its geometry with it, and every fallback in this header is the raw id — which is
// what the operator saw when a shell exited. Monotone on purpose: nothing here is ever cleared,
// because the last thing the node said about this pane stays the last thing anyone knows.
@Composable
private fun rememberLastKnown(paneId: String, info: PaneInfo?): PaneInfo? {
    var last by remember(paneId) { mutableStateOf(info) }
    if (info != null) last = info
    return last
}

// Everything the header says *about* the pane rather than which pane it is: what the frame stream
// is doing, what the keyboard is doing, what the agent is doing. Emitted as siblings so the row
// that hosts them decides how they lay out.
@Composable
private fun PaneMarks(pane: PaneState, info: PaneInfo?, readOnly: Boolean, gone: PaneGone?, shown: PaneView) {
    val tokens = Kampr.tokens
    if (gone != null) GoneBadge(gone)
    if (info?.detail != null) StreamBadge()
    if (pane.stale) StaleBadge(shown)
    if (pane.quiet) QuietBadge()
    if (pane.undelivered > 0) UnsentBadge(pane.undelivered)
    if (readOnly) {
        StatusBadge(
            "read-only", tokens.color.dim, tokens.color.surface,
            label = "This device is read-only — it cannot type into the pane",
        )
    }
    StatusChip(info)
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun PaneScreenMobile(
    pane: PaneState,
    info: PaneInfo?,
    view: PaneView,
    surfaces: PaneSurfaces,
    landscape: Boolean,
    readOnly: Boolean,
    gone: PaneGone? = null,
    onBack: () -> Unit,
    onView: (PaneView) -> Unit,
    onAnswer: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val density = LocalDensity.current
    val safe = LocalSafeArea.current
    // The terminal paints edge to edge and insets its scrollable content by whatever the chrome
    // above it takes. A guessed constant is a row of the grid hidden behind the bar with no
    // scroll left to reach it, so the number comes off the bar's own layout.
    //
    // Which is also why the status bar is paid for *here*, inside the bar's own background, rather
    // than by padding the screen: the bar's ground still runs under the clock so no grid shows
    // through it, the bar grows by exactly the inset, and the number the terminal is handed grows
    // with it. Padding the screen would letterbox the grid, which is the one thing it must not do.
    var chrome by remember { mutableStateOf<Dp?>(null) }
    val presence = rememberWatchPresence(pane.id, info)
    val shown = viewOn(info, view)
    val named = rememberLastKnown(pane.id, info)
    BoxWithConstraints(modifier.fillMaxSize().background(tokens.color.surface2)) {
        val guessed = chrome ?: if (landscape) 44.dp else 108.dp
        // Never more of the window than leaves a reply box standing. The terminal is unaffected:
        // it paints edge to edge and insets its own scrollable content by what it measures.
        val inset = guessed.coerceAtMost((maxHeight - REPLY_ROOM).coerceAtLeast(0.dp))
        CompositionLocalProvider(LocalPaneChrome provides chrome?.let(::PaneChrome)) {
            when (shown) {
                PaneView.Conversation -> surfaces.Conversation(
                    pane,
                    info,
                    Modifier.fillMaxSize().padding(top = inset),
                )
                else -> surfaces.Terminal(pane, info, Modifier.fillMaxSize())
            }
        }

        // The surface is painted first and the chrome floats over it, so composition order and
        // reading order disagree: without this a reader lands in the grid before the title.
        Column(
            Modifier
                .align(Alignment.TopStart)
                .fillMaxWidth()
                .readingOrder(-1f)
                .onGloballyPositioned { chrome = with(density) { it.size.height.toDp() } },
        ) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .background(tokens.color.bar)
                    .absolutePadding(
                        left = 16.dp + safe.left,
                        top = safe.top + if (landscape) 8.dp else 14.dp,
                        right = 16.dp + safe.right,
                        bottom = if (landscape) 8.dp else 11.dp,
                    ),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(11.dp),
            ) {
                BackAction("Back to the herd", onBack, target = if (landscape) LANDSCAPE_TOUCH else TOUCH)
                if (landscape) {
                    KText(paneName(named, pane), tokens.type.bodyStrong, tokens.color.text, Modifier.asHeading())
                    KText(geometryLine(named, pane, presence.others), tokens.type.meta, tokens.color.mute, Modifier.weight(1f))
                    // Leading the cluster the weighted line pins to the right edge, which is as far
                    // left as it goes here: ahead of it is Back and the pane title, and a zoom
                    // control wedged between a screen's back arrow and its heading reads as neither.
                    // It is also where the held slot costs nothing to look at — on the transcript
                    // the gap merges into the whitespace the geometry line already trails.
                    ZoomSlot(pane, surfaces, shown)
                    PaneMarks(pane, info, readOnly, gone, shown)
                    NewAction(pane.id)
                    PaneManageAction(pane.id)
                    // Landscape has no second row to hang these off, and an agent pane opens in
                    // Conversation: without them here the terminal is unreachable without rotating.
                    if (info.talks) ViewSwitch(shown, onView, Modifier.width(210.dp))
                } else {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        KText(paneName(named, pane), tokens.type.paneTitle, tokens.color.text, Modifier.asHeading())
                        KText(geometryLine(named, pane, presence.others), tokens.type.meta, tokens.color.mute)
                    }
                    // The marks go on the row below. At 480 dpi the phone is 360 dp wide and the
                    // title is the only elastic thing on this line, so it is handed whatever the
                    // fixed items leave: 69 px of the 133 px `kampr · claude` needs against a
                    // blocked agent, and 0 px once the pane was also stale and read-only.
                    NewAction(pane.id)
                    PaneManageAction(pane.id)
                }
            }
            if (!landscape) {
                // Flowing rather than a Row: stale, unsent, read-only and a status pill together
                // outrun 360 dp, and a switch squeezed to nothing is not a switch.
                FlowRow(
                    Modifier
                        .fillMaxWidth()
                        .background(tokens.color.bar)
                        .edgeBottom()
                        .absolutePadding(left = 16.dp + safe.left, right = 16.dp + safe.right, bottom = 11.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                    itemVerticalAlignment = Alignment.CenterVertically,
                ) {
                    PaneMarks(pane, info, readOnly, gone, shown)
                    // Left of the switch, which is the ask, but after the badges rather than ahead
                    // of them: the held slot has to sit somewhere, and leading the row it indents
                    // every badge off the margin the line above starts at whenever the transcript
                    // is up. Behind the badges the same gap is the space before a right-hand
                    // control, and once the row wraps it is a trailing gap nobody can see.
                    ZoomSlot(pane, surfaces, shown)
                    // Weighted rather than the landscape row's fixed 210 dp. It is already 211 dp
                    // on a 411 dp phone, and the width it would be pinned to is one the 360 dp
                    // phone cannot spare: there the row wraps and the switch has the line to
                    // itself, where filling it is 160 dp a segment against 105 dp stranded beside
                    // a dead margin. What the report asked for is that it stop *changing*, and the
                    // slot above is what does that.
                    if (info.talks) ViewSwitch(shown, onView, Modifier.weight(1f))
                }
            }
            gone?.let { GoneStrip(it, Modifier.fillMaxWidth()) }
        }

        // A pane that can never paint, in the space it was never going to fill. Gated on
        // `painted` rather than on the fault alone: a grid that arrived before the node lost its
        // herdr is still the last true thing about the pane, and `stale` is what says so.
        streamFault(pane, info)?.let {
            StreamNotice(it, Modifier.align(Alignment.Center).readingOrder(-0.4f).padding(24.dp))
        }

        // Over the surface, never in the chrome: the terminal insets its scrollable content by
        // the chrome it measures, so a notice that joined the bar would hide a row of the pane
        // behind the thing telling you about it.
        WatchNotice(
            presence,
            Modifier
                .align(Alignment.TopEnd)
                .readingOrder(-0.5f)
                .padding(top = (chrome ?: if (landscape) 44.dp else 108.dp) + 8.dp, end = 12.dp),
        )

        // The conversation surface renders its own prompt strip and reply box, so the terminal
        // chrome stands down rather than stacking a second one underneath it.
        // Nothing typed here can reach a pane the node no longer has, and a row of keys that
        // works perfectly and delivers nothing is the shape this codebase has paid for twice.
        if (shown != PaneView.Conversation && gone == null) {
            Column(Modifier.align(Alignment.BottomStart).fillMaxWidth().readingOrder(1f)) {
                if (!readOnly) pane.pending?.let {
                    PendingBar(it, answering(LocalConnectionStatus.current, pane.undelivered), onAnswer)
                }
                surfaces.KeyRow(pane, landscape, Modifier.fillMaxWidth())
            }
        }
    }
}

@Composable
private fun ViewSwitch(view: PaneView, onView: (PaneView) -> Unit, modifier: Modifier) {
    Segmented(
        listOf("Terminal", "Conversation"),
        if (view == PaneView.Conversation) 1 else 0,
        { onView(if (it == 1) PaneView.Conversation else PaneView.Terminal) },
        modifier,
        what = "view",
    )
}

// A frame that has stopped arriving is a fact about what is on the screen, and the eye gets it
// from a badge that a reader had no way to hear.
@Composable
private fun StaleBadge(shown: PaneView) {
    val tokens = Kampr.tokens
    // Named for the surface that is up. The grid says it a second way by washing out, and a
    // transcript says it no other way at all: a reader cannot date a message by looking at it, so
    // the badge is the whole signal and it may not be about a terminal nobody has open.
    val holding = when (shown) {
        PaneView.Terminal -> "showing the last grid"
        PaneView.Conversation -> "showing the last transcript that arrived"
        PaneView.Split -> "showing the last of both that arrived"
    }
    Box(Modifier.announce("Stale — this pane has stopped sending frames, $holding")) {
        StatusBadge("Stale", tokens.color.working, tokens.color.surface)
    }
}

// The badge for the state nothing could see. Worded as what is known rather than as a diagnosis,
// because the client cannot tell which half of the node stopped — only that the two halves
// disagree. `working` and not `blocked`: it is a warning about the picture, not a question waiting
// on the operator, and the pane may still be perfectly reachable by typing.
@Composable
private fun QuietBadge() {
    val tokens = Kampr.tokens
    Box(
        Modifier.announce(
            "Quiet — this node says the pane is active but has sent no frames; what is shown may be out of date",
            urgent = true,
        )
    ) {
        StatusBadge("Quiet", tokens.color.working, tokens.color.surface)
    }
}

// Stale is about the inbound stream. Nothing said the other half — that what is being typed is
// going nowhere — and silence there is what let 136 keystrokes vanish unnoticed.
@Composable
private fun UnsentBadge(count: Int) {
    val tokens = Kampr.tokens
    val what = if (count == 1) "1 keystroke" else "$count keystrokes"
    Box(
        Modifier.announce(
            "$what did not reach this pane — nothing you type is being delivered",
            urgent = true,
        )
    ) {
        StatusBadge("Not sent · $count", tokens.color.blocked, tokens.color.blockedBg)
    }
}

// Never the raw id. `pane.id` is a node ULID and a herdr coordinate, and a header that falls back
// to it has told the operator their pane is gone in the one vocabulary that reads as a fault in
// the app. The name a pane had outlives the herd entry; what says it is over is the strip.
private fun paneName(info: PaneInfo?, pane: PaneState): String =
    info?.let(::paneTitle) ?: pane.id.substringAfter('/')

private fun geometryLine(info: PaneInfo?, pane: PaneState, others: Int): String {
    // Off the pane's own id when the herd entry has gone with it. The fallback used to be the
    // whole id in the local column, which is the same node ULID the title was reported for.
    val id = info?.id ?: pane.id
    val node = id.substringBefore('/').take(8)
    val local = id.substringAfter('/')
    // A pane nobody has watched has no measured width, and the layout rect is not one — the node
    // omits `cols` rather than reporting a number no row was ever wrapped at.
    val size = info?.let { "${it.cols?.toString() ?: "—"}×${it.rows}" }
        ?: "${pane.cells.cols}×${pane.cells.rows}"
    // Only what is news. This line ellipsises at `… · observ…` on a 411 dp phone (#129), and the
    // constant that used to hold the last slot said the same thing about every pane — which one
    // operator read as a mode they were stuck in rather than as the absence of company.
    return listOfNotNull(node, local, size, watchersTag(others))
        .filter { it.isNotEmpty() }
        .joinToString(" · ")
}

// The strip appears because the agent asked something, not because anyone touched the screen, so
// it announces itself and carries the question a reader would otherwise have to go looking for.
@Composable
private fun PendingBar(pending: ServerMsg.Pending, answering: Answering, onAnswer: (String) -> Unit) {
    val tokens = Kampr.tokens
    val question = pending.question ?: "The agent is waiting for an answer"
    Column(
        Modifier
            .fillMaxWidth()
            .announce(
                "$question. ${pending.options.size} answers: " +
                    pending.options.joinToString(", ") { "${it.key} ${it.label}" },
                urgent = true,
            ),
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(start = 10.dp, top = 11.dp, end = 10.dp, bottom = 9.dp),
            horizontalArrangement = Arrangement.spacedBy(9.dp),
        ) {
            pending.options.forEachIndexed { index, option ->
                val primary = index == 0 && answering.enabled
                val shape = RoundedCornerShape(tokens.radii.md)
                Box(
                    Modifier
                        .background(if (primary) tokens.color.accent else tokens.color.surface, shape)
                        .edge(tokens.card, shape)
                        .touchable()
                        .action(
                            "Answer ${option.key}, ${option.label}",
                            { onAnswer(option.key) },
                            shape,
                            enabled = answering.enabled,
                        )
                        .padding(horizontal = 17.dp, vertical = 10.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    KText(
                        "${option.key} · ${option.label}",
                        tokens.type.buttonSmall,
                        when {
                            !answering.enabled -> tokens.color.mute
                            primary -> tokens.color.onAccent
                            else -> tokens.color.text
                        },
                    )
                }
            }
        }
        answering.note?.let {
            KText(
                it,
                tokens.type.micro,
                tokens.color.blocked,
                Modifier.padding(start = 10.dp, end = 10.dp, bottom = 9.dp),
                maxLines = 2,
            )
        }
    }
}

@Composable
fun PaneScreenDesktop(
    pane: PaneState,
    info: PaneInfo?,
    view: PaneView,
    surfaces: PaneSurfaces,
    readOnly: Boolean,
    gone: PaneGone? = null,
    onView: (PaneView) -> Unit,
    onAnswer: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val density = LocalDensity.current
    val safe = LocalSafeArea.current
    var chrome by remember { mutableStateOf<Dp?>(null) }
    val presence = rememberWatchPresence(pane.id, info)
    val shown = viewOn(info, view)
    val named = rememberLastKnown(pane.id, info)
    Box(modifier.fillMaxSize().background(tokens.color.surface2)) {
        CompositionLocalProvider(LocalPaneChrome provides chrome?.let(::PaneChrome)) {
            Row(Modifier.fillMaxSize()) {
                // Split shares the width rather than pinning the terminal to a fixed one: on a
                // wide monitor a fixed width crops the grid and leaves the other half empty.
                if (shown != PaneView.Conversation) {
                    surfaces.Terminal(pane, info, Modifier.weight(1f).fillMaxHeight())
                }
                if (shown != PaneView.Terminal) {
                    surfaces.Conversation(
                        pane,
                        info,
                        Modifier.weight(1f).fillMaxHeight().padding(top = chrome ?: 56.dp),
                    )
                }
            }
        }

        Column(
            Modifier
                .align(Alignment.TopStart)
                .fillMaxWidth()
                .readingOrder(-1f)
                .onGloballyPositioned { chrome = with(density) { it.size.height.toDp() } },
        ) {
        Row(
            Modifier
                .fillMaxWidth()
                .background(tokens.color.bar)
                .edgeBottom()
                .absolutePadding(left = 18.dp + safe.left, top = 13.dp + safe.top, right = 18.dp + safe.right, bottom = 13.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                KText(paneName(named, pane), tokens.type.paneTitle, tokens.color.text, Modifier.asHeading())
                KText(geometryLine(named, pane, presence.others), tokens.type.meta, tokens.color.mute)
            }
            if (gone != null) GoneBadge(gone)
            if (info?.detail != null) StreamBadge()
            if (pane.stale) StaleBadge(shown)
            if (pane.quiet) QuietBadge()
            if (pane.undelivered > 0) UnsentBadge(pane.undelivered)
            StatusChip(info)
            NewAction(pane.id)
            PaneManageAction(pane.id)
            // Closing the left-hand toolbar rather than trailing the switch: the desk has room to
            // put the two apart, and the slot it holds on the transcript is then a few pixels of a
            // spacer that was already empty.
            ZoomSlot(pane, surfaces, shown)
            Box(Modifier.weight(1f))
            if (info.talks) {
                // Terminal first and Split last, in the order a pane is actually opened in: the
                // switch led with the one view a pane never opens as, so the two that matter sat
                // where the eye arrives second.
                Segmented(
                    listOf("Terminal", "Conversation", "Split"),
                    when (shown) {
                        PaneView.Terminal -> 0
                        PaneView.Conversation -> 1
                        PaneView.Split -> 2
                    },
                    { index ->
                        onView(
                            when (index) {
                                0 -> PaneView.Terminal
                                1 -> PaneView.Conversation
                                else -> PaneView.Split
                            }
                        )
                    },
                    Modifier.width(320.dp),
                    what = "view",
                )
            }
        }
            gone?.let { GoneStrip(it, Modifier.fillMaxWidth()) }
        }

        streamFault(pane, info)?.let {
            StreamNotice(it, Modifier.align(Alignment.Center).readingOrder(-0.4f).padding(24.dp))
        }

        WatchNotice(
            presence,
            Modifier
                .align(Alignment.TopEnd)
                .readingOrder(-0.5f)
                .padding(top = (chrome ?: 56.dp) + 8.dp, end = 14.dp),
        )

        // The desktop breakpoint is not a desk. An Android tablet in landscape is 1280x800 dp,
        // which lands here, and it has no keys — so this layout offered no Escape, no Ctrl and no
        // arrow cluster to a device whose only keyboard is the one the app draws. What decides it
        // is therefore whether a keyboard is attached, and not how wide the window is.
        //
        // Terminal only, not Split: in Split the transcript has half the window and its own
        // composer at the bottom of it, and a full-width row of caps across both halves is a
        // second input surface laid over the first.
        if (shown == PaneView.Terminal && gone == null) {
            Column(Modifier.align(Alignment.BottomStart).fillMaxWidth().readingOrder(1f)) {
                if (!readOnly) pane.pending?.let {
                    PendingBar(it, answering(LocalConnectionStatus.current, pane.undelivered), onAnswer)
                }
                if (keyRowNeeded()) surfaces.KeyRow(pane, compact = true, Modifier.fillMaxWidth())
            }
        }
    }
}
