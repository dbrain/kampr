package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
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
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg

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
    // The terminal paints edge to edge and insets its scrollable content by whatever the chrome
    // above it takes. A guessed constant is a row of the grid hidden behind the bar with no
    // scroll left to reach it, so the number comes off the bar's own layout.
    var chrome by remember { mutableStateOf<Dp?>(null) }
    Box(modifier.fillMaxSize().background(tokens.color.surface2)) {
        CompositionLocalProvider(LocalPaneChrome provides chrome?.let(::PaneChrome)) {
            when (view) {
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
                    .padding(start = 16.dp, top = if (landscape) 8.dp else 14.dp, end = 16.dp, bottom = if (landscape) 8.dp else 11.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(11.dp),
            ) {
                BackAction("Back to the herd", onBack, target = if (landscape) LANDSCAPE_TOUCH else TOUCH)
                if (landscape) {
                    KText(info?.let(::paneTitle) ?: pane.id, tokens.type.bodyStrong, tokens.color.text, Modifier.asHeading())
                    KText(geometryLine(info, pane), tokens.type.meta, tokens.color.mute, Modifier.weight(1f))
                } else {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        KText(info?.let(::paneTitle) ?: pane.id, tokens.type.paneTitle, tokens.color.text, Modifier.asHeading())
                        KText(geometryLine(info, pane), tokens.type.meta, tokens.color.mute)
                    }
                }
                if (pane.stale) StaleBadge()
                if (pane.undelivered > 0) UnsentBadge(pane.undelivered)
                if (readOnly) {
                    StatusBadge(
                        "read-only", tokens.color.dim, tokens.color.surface,
                        label = "This device is read-only — it cannot type into the pane",
                    )
                }
                if (!landscape) TakingPaneAction(pane.id, info?.let(::paneTitle) ?: pane.id)
                NewAction(pane.id)
                PaneManageAction(pane.id)
                StatusChip(info)
                // Landscape has no second row to hang these off, and an agent pane opens in
                // Conversation: without them here the terminal is unreachable without rotating.
                if (landscape) {
                    ViewSwitch(view, onView, Modifier.width(210.dp))
                    surfaces.Zoom(pane, Modifier)
                }
            }
            if (!landscape) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .background(tokens.color.bar)
                        .edgeBottom()
                        .padding(start = 16.dp, end = 16.dp, bottom = 11.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    ViewSwitch(view, onView, Modifier.weight(1f))
                    surfaces.Zoom(pane, Modifier)
                }
            }
        }

        // The conversation surface renders its own prompt strip and reply box, so the terminal
        // chrome stands down rather than stacking a second one underneath it.
        if (view != PaneView.Conversation) {
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

private fun geometryLine(info: PaneInfo?, pane: PaneState): String {
    val node = info?.id?.substringBefore('/')?.take(8) ?: ""
    val local = info?.id?.substringAfter('/') ?: pane.id
    // A pane nobody has watched has no measured width, and the layout rect is not one — the node
    // omits `cols` rather than reporting a number no row was ever wrapped at.
    val size = info?.let { "${it.cols?.toString() ?: "—"}×${it.rows}" }
        ?: "${pane.cells.cols}×${pane.cells.rows}"
    return listOf(node, local, size, "observing").filter { it.isNotEmpty() }.joinToString(" · ")
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
    var chrome by remember { mutableStateOf<Dp?>(null) }
    Box(modifier.fillMaxSize().background(tokens.color.surface2)) {
        CompositionLocalProvider(LocalPaneChrome provides chrome?.let(::PaneChrome)) {
            Row(Modifier.fillMaxSize()) {
                // Split shares the width rather than pinning the terminal to a fixed one: on a
                // wide monitor a fixed width crops the grid and leaves the other half empty.
                if (view != PaneView.Conversation) {
                    surfaces.Terminal(pane, info, Modifier.weight(1f).fillMaxHeight())
                }
                if (view != PaneView.Terminal) {
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
                .padding(horizontal = 18.dp, vertical = 13.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                KText(info?.let(::paneTitle) ?: pane.id, tokens.type.paneTitle, tokens.color.text, Modifier.asHeading())
                KText(geometryLine(info, pane), tokens.type.meta, tokens.color.mute)
            }
            if (pane.stale) StaleBadge()
            if (pane.undelivered > 0) UnsentBadge(pane.undelivered)
            StatusChip(info)
            NewAction(pane.id)
            PaneManageAction(pane.id)
            Box(Modifier.weight(1f))
            Segmented(
                listOf("Split", "Terminal", "Conversation"),
                when (view) {
                    PaneView.Split -> 0
                    PaneView.Terminal -> 1
                    PaneView.Conversation -> 2
                },
                { index ->
                    onView(
                        when (index) {
                            0 -> PaneView.Split
                            1 -> PaneView.Terminal
                            else -> PaneView.Conversation
                        }
                    )
                },
                Modifier.width(320.dp),
                what = "view",
            )
        }

        if (!readOnly && view == PaneView.Terminal) {
            pane.pending?.let {
                Box(Modifier.align(Alignment.BottomStart).fillMaxWidth()) { PendingBar(it, onAnswer) }
            }
        }
    }
}
