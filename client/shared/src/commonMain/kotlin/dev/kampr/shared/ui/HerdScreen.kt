package dev.kampr.shared.ui

import androidx.compose.foundation.ScrollState
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
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Area
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Folds
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.NodeGroup
import dev.kampr.shared.model.TriageItem
import dev.kampr.shared.model.tree
import dev.kampr.shared.theme.Kampr

@Composable
fun HerdPortrait(
    herd: Herd,
    connection: ConnectionStatus,
    now: Double,
    localRtt: Double?,
    triage: List<TriageItem>,
    onOpenPane: (String) -> Unit,
    onApprove: ((String, String?) -> Unit)?,
    onResync: () -> Unit = {},
    scroll: ScrollState = rememberScrollState(),
    modifier: Modifier = Modifier,
    folds: Folds = Folds(),
    onFold: OnFold = NO_FOLD,
    onArea: OnArea = NO_AREA,
) {
    val tokens = Kampr.tokens
    val groups = herd.tree()
    var listing by remember { mutableStateOf(false) }
    Box(modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize().background(tokens.color.bg)) {
            Row(
                Modifier.fillMaxWidth().padding(start = 20.dp, top = 16.dp, end = 20.dp, bottom = 11.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                KText("Kampr", tokens.type.screenTitle, tokens.color.text, Modifier.asHeading())
                Row(verticalAlignment = Alignment.CenterVertically) {
                    NodeCountPill(herd.nodes.count { it.isServing }, connection, compact = true) { listing = true }
                    MosaicAction(LANDSCAPE_TOUCH)
                    NewAction(target = LANDSCAPE_TOUCH)
                }
            }
            PaletteBar(hint = null, modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 12.dp))
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
                Column(Modifier.weight(1f).verticalScroll(scroll)) {
                    groups.forEachIndexed { index, group ->
                        val ctx = HerdRowsCtx(HerdLook.Portrait, now, localRtt, folds, onFold, onOpenPane, onArea = onArea)
                        NodeHeader(
                            group,
                            ctx,
                            PaddingValues(start = 22.dp, end = 12.dp, top = if (index == 0) 2.dp else 10.dp, bottom = 4.dp),
                        )
                        GroupBody(group, ctx, Modifier.padding(horizontal = 16.dp))
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
    onApprove: ((String, String?) -> Unit)?,
    onResync: () -> Unit = {},
    scroll: ScrollState = rememberScrollState(),
    modifier: Modifier = Modifier,
    folds: Folds = Folds(),
    onFold: OnFold = NO_FOLD,
    onArea: OnArea = NO_AREA,
) {
    val tokens = Kampr.tokens
    val groups = herd.tree()
    var listing by remember { mutableStateOf(false) }
    Box(modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize().background(tokens.color.bg)) {
            Row(
                Modifier.fillMaxWidth().padding(start = 18.dp, top = 10.dp, end = 18.dp, bottom = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                KText("Kampr", tokens.type.paneTitle, tokens.color.text, Modifier.asHeading())
                NodeCountPill(herd.nodes.count { it.isServing }, connection, compact = true) { listing = true }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    MosaicAction(LANDSCAPE_TOUCH)
                    NewAction(target = LANDSCAPE_TOUCH)
                    PaletteAction(LANDSCAPE_TOUCH)
                }
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
                    val columns = groups.balancedColumns(plan.count, folds)
                    val ctx = HerdRowsCtx(HerdLook.Landscape, now, localRtt, folds, onFold, onOpenPane, onArea = onArea)
                    Row(
                        Modifier.fillMaxWidth().verticalScroll(scroll).padding(horizontal = 10.dp),
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
                                    NodeHeader(group, ctx, PaddingValues(start = 8.dp, end = 0.dp, top = 8.dp, bottom = 4.dp))
                                    GroupBody(group, ctx)
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

private fun List<NodeGroup>.balancedColumns(count: Int, folds: Folds): List<List<NodeGroup>> {
    val columns = List(count) { mutableListOf<NodeGroup>() }
    val heights = IntArray(count)
    for (group in this) {
        val shortest = heights.indices.minBy { heights[it] }
        columns[shortest] += group
        heights[shortest] += group.rows(folds)
    }
    return columns
}

private fun NodeGroup.rows(folds: Folds): Int {
    val listed = when (folds.area(this)) {
        Area.Terminals -> terminals.size
        Area.Containers -> node.containers.size
        Area.Muffit -> background.size
    }
    return listed + 1 + children.sumOf { it.rows(folds) }
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
    collapsed: Boolean = false,
    onCollapsed: (Boolean) -> Unit = {},
    scroll: ScrollState = rememberScrollState(),
    modifier: Modifier = Modifier,
    folds: Folds = Folds(),
    onFold: OnFold = NO_FOLD,
    onArea: OnArea = NO_AREA,
) {
    val tokens = Kampr.tokens
    val groups = herd.tree()
    var listing by remember { mutableStateOf(false) }
    if (collapsed) {
        HerdRail(
            herd = herd,
            connection = connection,
            now = now,
            activePaneId = activePaneId,
            deviceName = deviceName,
            deviceDetail = deviceDetail,
            onOpenPane = onOpenPane,
            onSettings = onSettings,
            onExpand = { onCollapsed(false) },
            modifier = modifier,
            folds = folds,
            onFold = onFold,
        )
        return
    }
    Box(modifier.width(SIDEBAR_WIDTH).fillMaxHeight()) {
        Column(
            Modifier
                .fillMaxSize()
                .background(tokens.color.bar)
                .readingOrder(-1f)
                .edgeEnd(),
        ) {
            // Two rows, because one will not hold them. The title, the machine pill and four
            // 36 dp controls measure ~279 dp against the 260 this sidebar has inside its own
            // padding, and `GlyphAction`'s `Modifier.size` coerces to the constraint it is
            // handed — so the row did not overflow visibly, it silently gave the whole deficit
            // to the last child and the + measured 0 x 0.
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(start = 18.dp, top = 14.dp, end = 12.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                KText("Kampr", tokens.type.screenTitle, tokens.color.text, Modifier.asHeading())
                GlyphAction(
                    KamprIcons.chevronLeft,
                    "Collapse the sidebar",
                    tokens.color.dim,
                    POINTER_TOUCH,
                    chip = 24.dp,
                ) { onCollapsed(true) }
            }
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(start = 18.dp, top = 8.dp, end = 12.dp, bottom = 12.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                NodeCountPill(herd.nodes.count { it.isServing }, connection, compact = true) { listing = true }
                Row(horizontalArrangement = Arrangement.spacedBy(2.dp), verticalAlignment = Alignment.CenterVertically) {
                    MosaicAction(POINTER_TOUCH)
                    FleetAction(POINTER_TOUCH)
                    NewAction(target = POINTER_TOUCH)
                }
            }
            PaletteBar(hint = "ctrl K", modifier = Modifier.padding(start = 14.dp, end = 14.dp, bottom = 12.dp))
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
                Column(Modifier.weight(1f).verticalScroll(scroll)) {
                    groups.forEachIndexed { index, group ->
                        val ctx = HerdRowsCtx(HerdLook.Sidebar, now, localRtt, folds, onFold, onOpenPane, activePaneId, onArea)
                        NodeHeader(
                            group,
                            ctx,
                            PaddingValues(start = 18.dp, end = 8.dp, top = if (index == 0) 0.dp else 12.dp, bottom = 3.dp),
                        )
                        GroupBody(group, ctx, Modifier.padding(horizontal = 10.dp))
                    }
                }
            }
            Row(
                Modifier
                    .fillMaxWidth()
                    .edgeTop()
                    .touchable()
                    .action("Settings — $deviceName, $deviceDetail", onSettings)
                    .padding(start = 14.dp, top = 12.dp, end = 14.dp, bottom = 14.dp + LocalSafeArea.current.bottom),
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

val SIDEBAR_WIDTH: Dp = 296.dp
