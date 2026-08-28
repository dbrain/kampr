package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.absolutePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.TriageItem
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.othersWatching
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.model.watchersPhrase
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

fun statusShape(status: AgentStatus): MarkShape = when (status) {
    AgentStatus.Blocked -> MarkShape.Square
    AgentStatus.Working -> MarkShape.Circle
    AgentStatus.Done -> MarkShape.Bar
    AgentStatus.Idle, AgentStatus.Unknown -> MarkShape.Ring
}

fun statusWord(status: AgentStatus): String = when (status) {
    AgentStatus.Blocked -> "Blocked"
    AgentStatus.Working -> "Working"
    AgentStatus.Done -> "Done"
    AgentStatus.Idle -> "Idle"
    AgentStatus.Unknown -> "No status"
}

// Colour, shape and — wherever a row is spoken — the word. The dot on its own was the whole of
// the encoding, and it is the one channel a colour-blind reader does not have.
@Composable
fun StatusMark(status: AgentStatus, size: Dp = 8.dp, modifier: Modifier = Modifier) {
    Mark(statusColor(status), statusShape(status), size, modifier.named(statusWord(status)))
}

fun paneSpoken(pane: PaneInfo, now: Double): String = listOfNotNull(
    paneTitle(pane),
    statusWord(statusOf(pane)),
    pane.cwd,
    "updated ${relativeTime(pane.updatedAt, now)}",
    watchersPhrase(othersWatching(pane)),
).joinToString(", ")

// One vocabulary for the socket wherever its state is said — the pill, the empty herd, the detail
// pane — so a device that never reached a node cannot be "Reconnecting" on one surface and silent
// on the next. Null is Live: the connection has no news of its own, and each surface says its own
// thing about a herd that is merely empty.
fun connectionWord(status: ConnectionStatus): String? = when (status) {
    is ConnectionStatus.Live -> null
    is ConnectionStatus.Offline -> "Reconnecting"
    is ConnectionStatus.Refused -> "Not paired with this node"
    ConnectionStatus.Connecting -> "Connecting"
    ConnectionStatus.Idle -> "Not connected"
}

// Refused is the one state that never comes back on its own, so it is the one that reads as a
// refusal rather than as weather.
@Composable
fun connectionColor(status: ConnectionStatus): Color {
    val color = Kampr.tokens.color
    return when (status) {
        is ConnectionStatus.Live -> color.done
        is ConnectionStatus.Refused -> color.blocked
        else -> color.working
    }
}

fun connectionShape(status: ConnectionStatus): MarkShape =
    if (status is ConnectionStatus.Live) MarkShape.Bar else MarkShape.Ring

// The count is the door to the list behind it: a pill that only ever said a number was a control
// with nothing under it, and "1 nodes" is what an unconditional plural reads as on a herd of one.
// The mark is the socket rather than the herd, because an unconditionally green dot told a device
// with no network at all that it was healthy and owned nothing.
@Composable
fun NodeCountPill(online: Int, connection: ConnectionStatus, compact: Boolean, onClick: () -> Unit) {
    val tokens = Kampr.tokens
    val counted = if (online == 1) "1 node" else "$online nodes"
    val word = connectionWord(connection)
    val tone = connectionColor(connection)
    Box(
        Modifier
            .touchable(if (compact) LANDSCAPE_TOUCH else TOUCH)
            .action(listOfNotNull("Machines — $counted online", word).joinToString(", "), onClick),
        contentAlignment = Alignment.Center,
    ) {
        Pill(
            horizontal = if (compact) 11.dp else 13.dp,
            vertical = if (compact) 5.dp else 7.dp,
        ) {
            Mark(tone, connectionShape(connection), 7.dp)
            KText(counted, tokens.type.pill, if (word == null) tokens.color.dim else tone)
        }
    }
}

// Four different pieces of news, and the operator can act on only some of them. Painting nothing
// at all said the healthiest of the four to the device that had the worst of them.
// A machine drawn with nothing under it, answering the question that leaves. `online` false on a
// local node means its herdr is stopped rather than the machine being gone — and a manage op sent
// there starts it (#324, #325), so the + in the bar is the answer and is worth naming: nothing
// else on this screen says that starting a workspace on a cold host is a thing that works.
@Composable
fun NodeQuiet(node: NodeInfo, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val said = when {
        !node.online && node.kind == "local" -> "herdr is not running on ${node.name} — New (+) starts it"
        !node.online -> node.detail ?: "this machine is not reachable"
        else -> "nothing running on this machine"
    }
    KText(said, tokens.type.captionSmall, tokens.color.mute, modifier.announce(said), maxLines = 2)
}

@Composable
fun HerdEmpty(connection: ConnectionStatus, compact: Boolean, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val headline = connectionWord(connection) ?: "No panes yet"
    val detail = when (connection) {
        is ConnectionStatus.Live -> "This node is up and has nothing running on it."
        is ConnectionStatus.Offline -> "This device has lost the node, and kept none of the herd."
        is ConnectionStatus.Refused -> "This node does not know this device. Pair it again from Settings."
        ConnectionStatus.Connecting -> "Reaching the node."
        ConnectionStatus.Idle -> "This device has not reached a node yet."
    }
    Column(
        modifier.fillMaxWidth().announce("$headline. $detail"),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(if (compact) 7.dp else 10.dp),
    ) {
        Mark(connectionColor(connection), connectionShape(connection), if (compact) 9.dp else 11.dp)
        KText(
            headline,
            if (compact) tokens.type.bodyStrong else tokens.type.paneTitle,
            tokens.color.text,
            maxLines = 2,
        )
        KText(
            detail,
            tokens.type.caption.copy(textAlign = TextAlign.Center),
            tokens.color.mute,
            Modifier.widthIn(max = 260.dp),
            maxLines = 3,
        )
    }
}

// A herd still on screen while the socket is down is a memory, not a status. The pane surfaces
// already spell that fact "Stale" for a single grid; this says it about the whole list.
@Composable
fun StaleHerdNote(connection: ConnectionStatus, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    Box(modifier) {
        StatusBadge(
            "Stale",
            tokens.color.working,
            tokens.color.surface,
            label = listOfNotNull(
                "Stale — these machines are the last this device saw, not the herd now",
                connectionWord(connection),
            ).joinToString(", "),
        )
    }
}

// How this client reaches a machine, in the one word the wire supports. `kind` is all the node
// says — `local` for the one this client is connected to, `peer` for anything it relays — and
// three surfaces used to answer "tailnet", which is a claim about the transport nobody measured
// and is wrong on a herd of LAN machines.
fun nodeReach(node: NodeInfo): String = if (node.kind == "local") "local" else "peer"

// One vocabulary for a machine wherever one is listed — the herd's own sheet and the settings
// screen — so a peer cannot be a "peer" on one surface and something else on the other.
// `withStatus` adds the one fact the eye takes from the mark beside the row and the ear takes
// from nothing at all.
fun nodeFacts(node: NodeInfo, withStatus: Boolean = false): List<String> = listOfNotNull(
    if (node.kind == "local") "this machine" else "peer",
    node.session.takeIf { it != "default" }?.let { "session $it" },
    if (withStatus) (if (node.online) "online" else "offline") else null,
    node.build?.let { "kampr $it" },
    node.update?.let { "$it available" },
    node.herdrVersion?.let { "herdr $it" },
    node.detail,
)

@Composable
fun NodeListSheet(
    nodes: List<NodeInfo>,
    breakpoint: Breakpoint,
    onResync: () -> Unit,
    onDismiss: () -> Unit,
) {
    val tokens = Kampr.tokens
    val compact = breakpoint != Breakpoint.Portrait
    val online = nodes.count { it.online }
    BottomSheet(breakpoint, onDismiss) {
        SheetHeader(
            title = "Machines",
            subtitle = if (nodes.isEmpty()) "none yet" else "$online of ${nodes.size} online",
            onBack = null,
            onClose = onDismiss,
            compact = compact,
        )
        Column(
            Modifier
                .weight(1f, fill = false)
                .verticalScroll(rememberScrollState())
                .padding(start = 20.dp, end = 20.dp, bottom = 20.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (nodes.isEmpty()) {
                KText(
                    "This device is not connected to a node yet.",
                    tokens.type.captionSmall,
                    tokens.color.mute,
                    maxLines = 2,
                )
            }
            // Reachability is painted as well as spoken here: this sheet exists to answer "is that
            // machine up", and `detail` is the node's own account of why it is not.
            for (node in nodes) {
                SheetCard(
                    icon = null,
                    iconTint = null,
                    title = node.name,
                    subtitle = nodeFacts(node, withStatus = true).joinToString(" · "),
                    compact = compact,
                    trailing = {
                        Mark(
                            if (node.online) tokens.color.done else tokens.color.blocked,
                            if (node.online) MarkShape.Bar else MarkShape.Ring,
                            9.dp,
                        )
                    },
                )
            }
            // `resync` is the protocol's recovery from a herd delta this client never saw, and
            // this list is where a missing machine is noticed. Nothing else in the client could
            // send one, which left the escape hatch bolted shut.
            QuietAction(
                "Refresh",
                onResync,
                Modifier.fillMaxWidth().padding(top = 4.dp),
                label = "Ask this node for the whole herd again",
            )
        }
    }
}

@Composable
fun NodeHeader(node: NodeInfo, measuredRtt: Double?, padding: PaddingValues) {
    val tokens = Kampr.tokens
    val reach = nodeReach(node)
    Row(
        Modifier
            .fillMaxWidth()
            .named("${node.name}, $reach, ${formatLatency(node.rttMs ?: measuredRtt)}")
            .padding(padding),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.Bottom,
    ) {
        LabelText(node.name, tokens.type.sectionLabel, tokens.color.text)
        KText("$reach · ${formatLatency(node.rttMs ?: measuredRtt)}", tokens.type.meta, tokens.color.mute)
    }
}

@Composable
fun PaneCard(pane: PaneInfo, now: Double, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val status = statusOf(pane)
    val quiet = status == AgentStatus.Idle || status == AgentStatus.Unknown
    val shape = RoundedCornerShape(tokens.radii.lg)
    Surface(
        modifier
            .action("Open ${paneSpoken(pane, now)}", onClick, shape)
    ) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 11.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Badge(36.dp, 16.dp, statusIcon(status), statusColor(status), statusWord(status))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                KText(
                    paneTitle(pane),
                    if (quiet) tokens.type.cardTitleQuiet else tokens.type.cardTitle,
                    if (quiet) tokens.color.dim else tokens.color.text,
                )
                KText(pane.cwd ?: "", tokens.type.meta, tokens.color.mute)
            }
            WatchersTag(othersWatching(pane))
            Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                if (!quiet) StatusMark(status, 8.dp)
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
            .touchable(LANDSCAPE_TOUCH)
            .action(
                "Open ${paneSpoken(pane, now)}",
                onClick,
                shape,
                role = Role.Tab,
                selected = active,
            )
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        StatusMark(status, 7.dp)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(1.dp)) {
            KText(
                paneTitle(pane),
                if (quiet) tokens.type.cardTitleQuiet else tokens.type.cardTitle,
                if (quiet) tokens.color.dim else tokens.color.text,
            )
            KText(pane.cwd ?: "", tokens.type.meta, tokens.color.mute)
        }
        WatchersTag(othersWatching(pane))
        KText(relativeTime(pane.updatedAt, now), tokens.type.micro, tokens.color.mute)
    }
}

@Composable
fun Badge(box: Dp, glyph: Dp, icon: Icon, tint: Color, label: String? = null) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Box(
        Modifier
            .size(box)
            .let { if (label != null) it.named(label) else it }
            .background(tokens.color.raise, shape)
            .edge(tokens.card, shape),
        contentAlignment = Alignment.Center,
    ) {
        IconGlyph(icon, glyph, tint)
    }
}

@Composable
fun StatusBadge(text: String, tone: Color, background: Color, label: String? = null) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.pill)
    Box(
        Modifier
            .let { if (label != null) it.named(label) else it }
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
    // It arrives without the reader asking — an agent blocks while they are somewhere else on the
    // screen — and it is the one thing on this surface with a deadline attached to it.
    Column(
        modifier
            .fillMaxWidth()
            .announce(
                if (items.size > 1) "Needs you: ${items.size} agents are blocked"
                else "Needs you: ${paneTitle(items.first().pane)} is blocked",
                urgent = true,
            ),
        verticalArrangement = Arrangement.spacedBy(if (compact) 6.dp else 9.dp),
    ) {
        if (items.size > 1) {
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Mark(tokens.color.blocked, MarkShape.Square, 8.dp)
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
                if (!compact) Mark(tokens.color.blocked, MarkShape.Square, 8.dp)
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
                PrimaryAction(
                    "Open", onOpen, Modifier.weight(1f), tokens.type.buttonSmall, 10.dp,
                    label = "Open ${paneTitle(pane)}",
                )
                if (onApprove != null) {
                    QuietAction(
                        "Approve", onApprove, Modifier.weight(1f),
                        label = "Approve, and answer ${paneTitle(pane)} with option 1",
                    )
                }
            }
        }
    }
}

@Composable
fun BottomNav(selected: Tab, onSelect: (Tab) -> Unit) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    // The bar's *ground* still runs to the bottom of the screen — it is what the gesture handle
    // needs something behind it — but its labels stop above the strip the system draws in.
    Row(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeTop()
            .readingOrder(1f)
            .absolutePadding(
                left = safe.left,
                top = 5.dp,
                right = safe.right,
                bottom = 10.dp + safe.bottom,
            ),
    ) {
        NavItem(Tab.Herd, "Herd", KamprIcons.herd, selected, onSelect)
        NavItem(Tab.Settings, "Settings", KamprIcons.gear, selected, onSelect)
    }
}

@Composable
private fun RowScope.NavItem(tab: Tab, label: String, icon: Icon, selected: Tab, onSelect: (Tab) -> Unit) {
    val tokens = Kampr.tokens
    val active = tab == selected
    val tint = if (active) tokens.color.accent else tokens.color.mute
    Column(
        Modifier
            .weight(1f)
            .touchable()
            .action("$label tab", { onSelect(tab) }, role = Role.Tab, selected = active)
            .padding(vertical = 5.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        IconGlyph(icon, 19.dp, tint)
        KText(label, if (active) tokens.type.micro.copy(fontWeight = tokens.label.weight) else tokens.type.micro, tint)
    }
}
