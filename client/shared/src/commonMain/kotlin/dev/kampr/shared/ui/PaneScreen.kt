package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.model.watchersTag
import dev.kampr.shared.platform.LocalHardKeyboard
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg

// `has_conversation` is the node saying a transcript resolves for this pane, not that the pane is
// an agent — a freshly started `claude` reports false until its journal appears. It moves during a
// session in both directions, so it is read on every composition rather than at open.
private val PaneInfo?.talks: Boolean get() = this?.hasConversation == true

// What is actually rendered, against what was asked for. A remembered preference, a deep link and
// the desktop's own Split default all outlive the transcript that justified them, and the pane they
// name is still live — so the terminal is where that lands, and the request is kept rather than
// rewritten, which is what brings the operator's choice back when the transcript returns.
private fun viewOn(info: PaneInfo?, view: PaneView): PaneView =
    if (info.talks) view else PaneView.Terminal

// Why this pane will never paint, when there is nothing on its surface to read instead.
private fun streamFault(pane: PaneState, info: PaneInfo?): String? =
    info?.detail?.takeUnless { pane.painted }

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

// Everything the header says *about* the pane rather than which pane it is: what the frame stream
// is doing, what the keyboard is doing, what the agent is doing. Emitted as siblings so the row
// that hosts them decides how they lay out.
@Composable
private fun PaneMarks(pane: PaneState, info: PaneInfo?, readOnly: Boolean) {
    val tokens = Kampr.tokens
    if (info?.detail != null) StreamBadge()
    if (pane.stale) StaleBadge()
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
    Box(modifier.fillMaxSize().background(tokens.color.surface2)) {
        CompositionLocalProvider(LocalPaneChrome provides chrome?.let(::PaneChrome)) {
            when (shown) {
                PaneView.Conversation -> surfaces.Conversation(
                    pane,
                    info,
                    Modifier.fillMaxSize().padding(top = chrome ?: if (landscape) 44.dp else 108.dp),
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
                    KText(info?.let(::paneTitle) ?: pane.id, tokens.type.bodyStrong, tokens.color.text, Modifier.asHeading())
                    KText(geometryLine(info, pane, presence.others), tokens.type.meta, tokens.color.mute, Modifier.weight(1f))
                    PaneMarks(pane, info, readOnly)
                    NewAction(pane.id)
                    PaneManageAction(pane.id)
                    // Landscape has no second row to hang these off, and an agent pane opens in
                    // Conversation: without them here the terminal is unreachable without rotating.
                    if (info.talks) ViewSwitch(shown, onView, Modifier.width(210.dp))
                    // The zoom sheet belongs to the terminal surface, so the control that opens
                    // it goes wherever that surface is: on the transcript it opens nothing.
                    if (shown != PaneView.Conversation) surfaces.Zoom(pane, Modifier)
                } else {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        KText(info?.let(::paneTitle) ?: pane.id, tokens.type.paneTitle, tokens.color.text, Modifier.asHeading())
                        KText(geometryLine(info, pane, presence.others), tokens.type.meta, tokens.color.mute)
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
                    PaneMarks(pane, info, readOnly)
                    if (info.talks) ViewSwitch(shown, onView, Modifier.weight(1f)) else Box(Modifier.weight(1f))
                    if (shown != PaneView.Conversation) surfaces.Zoom(pane, Modifier)
                }
            }
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
        if (shown != PaneView.Conversation) {
            Column(Modifier.align(Alignment.BottomStart).fillMaxWidth().readingOrder(1f)) {
                if (!readOnly) pane.pending?.let { PendingBar(it, onAnswer) }
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
private fun StaleBadge() {
    val tokens = Kampr.tokens
    Box(Modifier.announce("Stale — this pane has stopped sending frames, showing the last grid")) {
        StatusBadge("Stale", tokens.color.working, tokens.color.surface)
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

private fun geometryLine(info: PaneInfo?, pane: PaneState, others: Int): String {
    val node = info?.id?.substringBefore('/')?.take(8) ?: ""
    val local = info?.id?.substringAfter('/') ?: pane.id
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
private fun PendingBar(pending: ServerMsg.Pending, onAnswer: (String) -> Unit) {
    val tokens = Kampr.tokens
    val question = pending.question ?: "The agent is waiting for an answer"
    Row(
        Modifier
            .fillMaxWidth()
            .announce(
                "$question. ${pending.options.size} answers: " +
                    pending.options.joinToString(", ") { "${it.key} ${it.label}" },
                urgent = true,
            )
            .horizontalScroll(rememberScrollState())
            .padding(start = 10.dp, top = 11.dp, end = 10.dp, bottom = 9.dp),
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        pending.options.forEachIndexed { index, option ->
            val shape = RoundedCornerShape(tokens.radii.md)
            Box(
                Modifier
                    .background(if (index == 0) tokens.color.accent else tokens.color.surface, shape)
                    .edge(if (index == 0) tokens.card else tokens.card, shape)
                    .touchable()
                    .action("Answer ${option.key}, ${option.label}", { onAnswer(option.key) }, shape)
                    .padding(horizontal = 17.dp, vertical = 10.dp),
                contentAlignment = Alignment.Center,
            ) {
                KText(
                    "${option.key} · ${option.label}",
                    tokens.type.buttonSmall,
                    if (index == 0) tokens.color.onAccent else tokens.color.text,
                )
            }
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

        Row(
            Modifier
                .align(Alignment.TopStart)
                .fillMaxWidth()
                .background(tokens.color.bar)
                .edgeBottom()
                .readingOrder(-1f)
                .onGloballyPositioned { chrome = with(density) { it.size.height.toDp() } }
                .absolutePadding(left = 18.dp + safe.left, top = 13.dp + safe.top, right = 18.dp + safe.right, bottom = 13.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                KText(info?.let(::paneTitle) ?: pane.id, tokens.type.paneTitle, tokens.color.text, Modifier.asHeading())
                KText(geometryLine(info, pane, presence.others), tokens.type.meta, tokens.color.mute)
            }
            if (info?.detail != null) StreamBadge()
            if (pane.stale) StaleBadge()
            if (pane.undelivered > 0) UnsentBadge(pane.undelivered)
            StatusChip(info)
            NewAction(pane.id)
            PaneManageAction(pane.id)
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
            if (shown != PaneView.Conversation) surfaces.Zoom(pane, Modifier)
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
        if (shown == PaneView.Terminal) {
            Column(Modifier.align(Alignment.BottomStart).fillMaxWidth().readingOrder(1f)) {
                if (!readOnly) pane.pending?.let { PendingBar(it, onAnswer) }
                if (!LocalHardKeyboard.current) surfaces.KeyRow(pane, compact = true, Modifier.fillMaxWidth())
            }
        }
    }
}
