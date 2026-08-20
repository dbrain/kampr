package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.TriageItem
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.util.formatLatency
import dev.kampr.shared.util.relativeTime
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo

@Composable
fun statusColor(status: AgentStatus): Color {
    val color = Kampr.tokens.color
    return when (status) {
        AgentStatus.Blocked -> color.blocked
        AgentStatus.Working -> color.working
        AgentStatus.Done -> color.done
        AgentStatus.Idle, AgentStatus.Unknown -> color.idle
    }
}

fun statusIcon(status: AgentStatus): Icon = when (status) {
    AgentStatus.Blocked -> KamprIcons.blockedAgent
    AgentStatus.Working -> KamprIcons.workingClock
    AgentStatus.Done -> KamprIcons.done
    AgentStatus.Idle, AgentStatus.Unknown -> KamprIcons.shell
}

@Composable
fun NodeCountPill(online: Int, compact: Boolean) {
    val tokens = Kampr.tokens
    Pill(horizontal = if (compact) 11.dp else 13.dp, vertical = if (compact) 5.dp else 7.dp) {
        Dot(tokens.color.done, 7.dp)
        KText("$online nodes", tokens.type.pill, tokens.color.dim)
    }
}

@Composable
fun NodeHeader(node: NodeInfo, measuredRtt: Double?, padding: PaddingValues) {
    val tokens = Kampr.tokens
    Row(
        Modifier.fillMaxWidth().padding(padding),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.Bottom,
    ) {
        LabelText(node.name, tokens.type.sectionLabel, tokens.color.text)
        val transport = if (node.kind == "local") "local" else "tailnet"
        KText("$transport · ${formatLatency(node.rttMs ?: measuredRtt)}", tokens.type.meta, tokens.color.mute)
    }
}

@Composable
fun PaneCard(pane: PaneInfo, now: Double, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val status = statusOf(pane)
    val quiet = status == AgentStatus.Idle || status == AgentStatus.Unknown
    Surface(modifier.alpha(if (quiet) 0.72f else 1f).clickable(onClick = onClick)) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 11.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Badge(36.dp, 16.dp, statusIcon(status), statusColor(status))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                KText(
                    paneTitle(pane),
                    if (quiet) tokens.type.cardTitleQuiet else tokens.type.cardTitle,
                    if (quiet) tokens.color.dim else tokens.color.text,
                )
                KText(pane.cwd ?: "", tokens.type.meta, tokens.color.mute)
            }
            Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                if (!quiet) Dot(statusColor(status), 8.dp)
                KText(relativeTime(pane.updatedAt, now), tokens.type.micro, tokens.color.mute)
            }
        }
    }
}

@Composable
fun PaneRow(pane: PaneInfo, now: Double, active: Boolean, onClick: () -> Unit) {
    val tokens = Kampr.tokens
    val status = statusOf(pane)
    val quiet = status == AgentStatus.Idle || status == AgentStatus.Unknown
    val shape = RoundedCornerShape(tokens.radii.sm)
    Row(
        Modifier
            .fillMaxWidth()
            .let { if (active) it.background(tokens.color.raise, shape) else it }
            .clickable(onClick = onClick)
            .alpha(if (quiet) 0.68f else 1f)
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Dot(statusColor(status), 7.dp, hollow = quiet)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(1.dp)) {
            KText(
                paneTitle(pane),
                if (quiet) tokens.type.cardTitleQuiet else tokens.type.cardTitle,
                if (quiet) tokens.color.dim else tokens.color.text,
            )
            KText(pane.cwd ?: "", tokens.type.meta, tokens.color.mute)
        }
        KText(relativeTime(pane.updatedAt, now), tokens.type.micro, tokens.color.mute)
    }
}

@Composable
fun Badge(box: Dp, glyph: Dp, icon: Icon, tint: Color) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Box(
        Modifier.size(box).background(tokens.color.raise, shape).edge(tokens.card, shape),
        contentAlignment = Alignment.Center,
    ) {
        IconGlyph(icon, glyph, tint)
    }
}

@Composable
fun StatusBadge(text: String, tone: Color, background: Color) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.pill)
    Box(
        Modifier
            .background(background, shape)
            .edge(tokens.card, shape)
            .padding(horizontal = 11.dp, vertical = 5.dp),
    ) {
        KText(text, tokens.type.badge, tone)
    }
}

// The triage list. "NEEDS YOU" first, above the herd and before anything else on the screen —
// a blocked agent is the only thing on this surface with a deadline attached to it.
@Composable
fun TriageList(
    items: List<TriageItem>,
    compact: Boolean,
    onOpen: (String) -> Unit,
    onApprove: ((String) -> Unit)?,
    modifier: Modifier = Modifier,
) {
    if (items.isEmpty()) return
    val tokens = Kampr.tokens
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(if (compact) 6.dp else 9.dp)) {
        if (items.size > 1) {
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Dot(tokens.color.blocked, 8.dp)
                LabelText(
                    "Needs you · ${items.size}",
                    tokens.type.caption.copy(
                        fontWeight = tokens.label.weight,
                        letterSpacing = tokens.label.tracking,
                    ),
                    tokens.color.blocked,
                )
            }
        }
        // Bounded, because a herd that has gone badly wrong must not push the herd itself off the
        // screen. The rest are still in the list below, with their own blocked dots.
        for (item in items.take(TRIAGE_SHOWN)) {
            BlockedNotice(
                pane = item.pane,
                question = item.question,
                compact = compact,
                label = if (items.size > 1) null else if (compact) "Needs you · 1" else "Needs you",
                onOpen = { onOpen(item.pane.id) },
                onApprove = onApprove?.let { approve -> { approve(item.pane.id) } },
            )
        }
        if (items.size > TRIAGE_SHOWN) {
            KText(
                "and ${items.size - TRIAGE_SHOWN} more below",
                tokens.type.micro,
                tokens.color.mute,
            )
        }
    }
}

private const val TRIAGE_SHOWN = 3

@Composable
fun BlockedNotice(
    pane: PaneInfo,
    question: String?,
    compact: Boolean,
    onOpen: () -> Unit,
    onApprove: (() -> Unit)?,
    label: String? = if (compact) "Needs you · 1" else "Needs you",
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(if (compact) tokens.radii.md else tokens.radii.lg)
    Column(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.blockedBg, shape)
            .border(1.dp, tokens.color.blocked, shape)
            .padding(horizontal = if (compact) 13.dp else 16.dp, vertical = if (compact) 11.dp else 14.dp),
        verticalArrangement = Arrangement.spacedBy(if (compact) 6.dp else 9.dp),
    ) {
        if (label != null) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (!compact) Dot(tokens.color.blocked, 8.dp)
                LabelText(
                    label,
                    tokens.type.caption.copy(fontWeight = tokens.label.weight, letterSpacing = tokens.label.tracking),
                    tokens.color.blocked,
                )
            }
        }
        if (compact) {
            KText(paneTitle(pane), tokens.type.bodyStrong, tokens.color.text)
            KText(question ?: "Waiting for an answer", tokens.type.captionSmall, tokens.color.dim)
        } else {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(13.dp),
            ) {
                Badge(40.dp, 19.dp, KamprIcons.blockedAgent, tokens.color.blocked)
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    KText(paneTitle(pane), tokens.type.paneTitle, tokens.color.text)
                    KText(question ?: "Waiting for an answer", tokens.type.caption, tokens.color.dim)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(9.dp)) {
                PrimaryAction("Open", onOpen, Modifier.weight(1f), tokens.type.buttonSmall, 10.dp)
                if (onApprove != null) QuietAction("Approve", onApprove, Modifier.weight(1f))
            }
        }
    }
}

@Composable
fun BottomNav(selected: Tab, onSelect: (Tab) -> Unit) {
    val tokens = Kampr.tokens
    Row(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeTop()
            .padding(top = 5.dp, bottom = 10.dp),
    ) {
        NavItem(Tab.Herd, "Herd", KamprIcons.herd, selected, onSelect)
        NavItem(Tab.Pane, "Pane", KamprIcons.pane, selected, onSelect)
        NavItem(Tab.Nodes, "Nodes", KamprIcons.nodes, selected, onSelect)
    }
}

@Composable
private fun RowScope.NavItem(tab: Tab, label: String, icon: Icon, selected: Tab, onSelect: (Tab) -> Unit) {
    val tokens = Kampr.tokens
    val active = tab == selected
    val tint = if (active) tokens.color.accent else tokens.color.mute
    Column(
        Modifier.weight(1f).clickable { onSelect(tab) }.padding(vertical = 5.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        IconGlyph(icon, 19.dp, tint)
        KText(label, if (active) tokens.type.micro.copy(fontWeight = tokens.label.weight) else tokens.type.micro, tint)
    }
}
