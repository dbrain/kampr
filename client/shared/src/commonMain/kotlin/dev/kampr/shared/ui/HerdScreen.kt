package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.NodeGroup
import dev.kampr.shared.model.TriageItem
import dev.kampr.shared.model.groups
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.PaneInfo

@Composable
fun HerdPortrait(
    herd: Herd,
    connection: ConnectionStatus,
    now: Double,
    localRtt: Double?,
    triage: List<TriageItem>,
    onOpenPane: (String) -> Unit,
    onApprove: ((String) -> Unit)?,
    onResync: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val groups = herd.groups()
    var listing by remember { mutableStateOf(false) }
    Box(modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize().background(tokens.color.bg)) {
            Row(
                Modifier.fillMaxWidth().padding(start = 20.dp, top = 16.dp, end = 20.dp, bottom = 11.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                KText("Herd", tokens.type.screenTitle, tokens.color.text, Modifier.asHeading())
                Row(horizontalArrangement = Arrangement.spacedBy(9.dp), verticalAlignment = Alignment.CenterVertically) {
                    NodeCountPill(herd.nodes.count { it.online }, connection, compact = false) { listing = true }
                    MosaicAction()
                    FleetAction()
                    NewAction()
                }
            }
            if (herd.stale && groups.isNotEmpty()) {
                StaleHerdNote(connection, Modifier.padding(start = 20.dp, end = 20.dp, bottom = 11.dp))
            }
            if (triage.isNotEmpty()) {
                Box(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 14.dp)) {
                    TriageList(triage, compact = false, onOpen = onOpenPane, onApprove = onApprove)
                }
            }
            if (groups.isEmpty()) {
                Box(
                    Modifier.weight(1f).fillMaxWidth().padding(horizontal = 24.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    HerdEmpty(connection, compact = false)
                }
            } else {
                Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                    groups.forEachIndexed { index, group ->
                        NodeHeader(
                            group.node,
                            localRtt,
                            PaddingValues(start = 22.dp, end = 22.dp, top = if (index == 0) 2.dp else 14.dp, bottom = 8.dp),
                        )
                        Column(
                            Modifier.padding(horizontal = 16.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            if (group.panes.isEmpty()) {
                                NodeQuiet(group.node, Modifier.padding(start = 6.dp, bottom = 4.dp))
                            }
                            for (pane in group.panes) {
                                PaneCard(pane, now, { onOpenPane(pane.id) }, Modifier.fillMaxWidth())
                            }
                        }
                    }
                    Box(Modifier.height(16.dp))
                }
            }
        }
        if (listing) NodeListSheet(herd.nodes, Breakpoint.Portrait, onResync) { listing = false }
    }
}

@Composable
fun HerdLandscape(
    herd: Herd,
    connection: ConnectionStatus,
    now: Double,
    localRtt: Double?,
    triage: List<TriageItem>,
    onOpenPane: (String) -> Unit,
    onApprove: ((String) -> Unit)?,
    onResync: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val groups = herd.groups()
    var listing by remember { mutableStateOf(false) }
    Box(modifier.fillMaxSize()) {
    Column(Modifier.fillMaxSize().background(tokens.color.bg)) {
            Row(
                Modifier.fillMaxWidth().padding(start = 18.dp, top = 10.dp, end = 18.dp, bottom = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                KText("Herd", tokens.type.paneTitle, tokens.color.text, Modifier.asHeading())
                NodeCountPill(herd.nodes.count { it.online }, connection, compact = true) { listing = true }
                MosaicAction(LANDSCAPE_TOUCH)
                FleetAction(LANDSCAPE_TOUCH)
                NewAction(target = LANDSCAPE_TOUCH)
                Box(Modifier.weight(1f))
                if (triage.isNotEmpty()) {
                    StatusBadge(
                        if (triage.size > 1) "Needs you · ${triage.size}" else "Needs you",
                        tokens.color.blocked,
                        tokens.color.blockedBg,
                        label = if (triage.size > 1) "${triage.size} agents need you" else "One agent needs you",
                    )
                }
            }
            if (herd.stale && groups.isNotEmpty()) {
                StaleHerdNote(connection, Modifier.padding(start = 18.dp, end = 18.dp, bottom = 6.dp))
            }
            if (groups.isEmpty()) {
                Box(
                    Modifier.weight(1f).fillMaxWidth().padding(horizontal = 24.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    HerdEmpty(connection, compact = true)
                }
            } else {
                BoxWithConstraints(Modifier.weight(1f)) {
                    val plan = columnPlan(maxWidth - 20.dp, COLUMN_GAP, wanted = groups.size)
                    val columns = groups.balancedColumns(plan.count)
                    Row(
                        Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 10.dp),
                        horizontalArrangement = Arrangement.spacedBy(COLUMN_GAP, Alignment.CenterHorizontally),
                    ) {
                        for (column in columns) {
                            Column(Modifier.width(plan.width)) {
                                if (column === columns.first() && triage.isNotEmpty()) {
                                    Box(Modifier.padding(horizontal = 6.dp, vertical = 4.dp)) {
                                        TriageList(triage, compact = true, onOpen = onOpenPane, onApprove = null)
                                    }
                                }
                                for (group in column) {
                                    NodeHeader(group.node, localRtt, PaddingValues(start = 8.dp, end = 8.dp, top = 10.dp, bottom = 6.dp))
                                    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                        if (group.panes.isEmpty()) {
                                            NodeQuiet(group.node, Modifier.padding(start = 8.dp, bottom = 4.dp))
                                        }
                                        for (pane in group.panes) {
                                            PaneCard(pane, now, { onOpenPane(pane.id) }, Modifier.fillMaxWidth())
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if (listing) NodeListSheet(herd.nodes, Breakpoint.Landscape, onResync) { listing = false }
    }
}

private val COLUMN_GAP = 10.dp

private fun List<NodeGroup>.balancedColumns(count: Int): List<List<NodeGroup>> {
    val columns = List(count) { mutableListOf<NodeGroup>() }
    val heights = IntArray(count)
    for (group in this) {
        val shortest = heights.indices.minBy { heights[it] }
        columns[shortest] += group
        heights[shortest] += group.panes.size + 1
    }
    return columns
}

@Composable
fun HerdSidebar(
    herd: Herd,
    connection: ConnectionStatus,
    now: Double,
    localRtt: Double?,
    triage: List<TriageItem>,
    activePaneId: String?,
    deviceName: String,
    deviceDetail: String,
    onOpenPane: (String) -> Unit,
    onSettings: () -> Unit,
    onResync: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val groups = herd.groups()
    var listing by remember { mutableStateOf(false) }
    Box(modifier.width(SIDEBAR_WIDTH).fillMaxHeight()) {
    Column(
            Modifier
                .fillMaxSize()
                .background(tokens.color.bar)
                .readingOrder(-1f)
                .edgeEnd(),
        ) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(start = 18.dp, top = 18.dp, end = 18.dp, bottom = 14.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                KText("Kampr", tokens.type.screenTitle, tokens.color.text, Modifier.asHeading())
                Row(horizontalArrangement = Arrangement.spacedBy(9.dp), verticalAlignment = Alignment.CenterVertically) {
                    NodeCountPill(herd.nodes.count { it.online }, connection, compact = true) { listing = true }
                    MosaicAction(LANDSCAPE_TOUCH)
                    FleetAction(LANDSCAPE_TOUCH)
                    NewAction(target = LANDSCAPE_TOUCH)
                }
            }
            if (herd.stale && groups.isNotEmpty()) {
                StaleHerdNote(connection, Modifier.padding(start = 14.dp, end = 14.dp, bottom = 12.dp))
            }
            if (triage.isNotEmpty()) {
                Box(Modifier.padding(start = 14.dp, end = 14.dp, bottom = 14.dp)) {
                    TriageList(triage, compact = true, onOpen = onOpenPane, onApprove = null)
                }
            }
            if (groups.isEmpty()) {
                Box(
                    Modifier.weight(1f).fillMaxWidth().padding(horizontal = 18.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    HerdEmpty(connection, compact = true)
                }
            } else {
                Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                    groups.forEachIndexed { index, group ->
                        NodeHeader(
                            group.node,
                            localRtt,
                            PaddingValues(start = 18.dp, end = 18.dp, top = if (index == 0) 0.dp else 16.dp, bottom = 7.dp),
                        )
                        Column(Modifier.padding(horizontal = 10.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                            if (group.panes.isEmpty()) {
                                NodeQuiet(group.node, Modifier.padding(start = 8.dp, bottom = 4.dp))
                            }
                            for (pane in group.panes) {
                                PaneRow(pane, now, pane.id == activePaneId) { onOpenPane(pane.id) }
                            }
                        }
                    }
                }
            }
            Row(
                Modifier
                    .fillMaxWidth()
                    .edgeTop()
                    .touchable()
                    .action("Settings — $deviceName, $deviceDetail", onSettings)
                    .padding(start = 14.dp, top = 12.dp, end = 14.dp, bottom = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(9.dp),
            ) {
                Box(
                    Modifier.size(26.dp).background(tokens.color.raise, RoundedCornerShape(tokens.radii.sm)),
                    contentAlignment = Alignment.Center,
                ) {
                    IconGlyph(KamprIcons.lockSmall, 13.dp, tokens.color.dim)
                }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(1.dp)) {
                    KText(deviceName, tokens.type.captionSmall.copy(fontWeight = tokens.label.weight), tokens.color.text)
                    KText(deviceDetail, tokens.type.micro, tokens.color.mute)
                }
                IconGlyph(KamprIcons.gear, 14.dp, tokens.color.mute)
            }
        }
        if (listing) NodeListSheet(herd.nodes, Breakpoint.Desktop, onResync) { listing = false }
    }
}

private val SIDEBAR_WIDTH = 296.dp
