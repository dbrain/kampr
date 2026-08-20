package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.NodeGroup
import dev.kampr.shared.model.TriageItem
import dev.kampr.shared.model.groups
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.PaneInfo

@Composable
fun HerdPortrait(
    herd: Herd,
    now: Double,
    localRtt: Double?,
    triage: List<TriageItem>,
    onOpenPane: (String) -> Unit,
    onApprove: ((String) -> Unit)?,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        Row(
            Modifier.fillMaxWidth().padding(start = 20.dp, top = 16.dp, end = 20.dp, bottom = 11.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            KText("Herd", tokens.type.screenTitle, tokens.color.text)
            Row(horizontalArrangement = Arrangement.spacedBy(9.dp), verticalAlignment = Alignment.CenterVertically) {
                NodeCountPill(herd.nodes.count { it.online }, compact = false)
                NewAction()
            }
        }
        if (triage.isNotEmpty()) {
            Box(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 14.dp)) {
                TriageList(triage, compact = false, onOpen = onOpenPane, onApprove = onApprove)
            }
        }
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            herd.groups().forEachIndexed { index, group ->
                NodeHeader(
                    group.node,
                    localRtt,
                    PaddingValues(start = 22.dp, end = 22.dp, top = if (index == 0) 2.dp else 14.dp, bottom = 8.dp),
                )
                Column(
                    Modifier.padding(horizontal = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    for (pane in group.panes) {
                        PaneCard(pane, now, { onOpenPane(pane.id) }, Modifier.fillMaxWidth())
                    }
                }
            }
            Box(Modifier.height(16.dp))
        }
    }
}

@Composable
fun HerdLandscape(
    herd: Herd,
    now: Double,
    localRtt: Double?,
    triage: List<TriageItem>,
    onOpenPane: (String) -> Unit,
    onApprove: ((String) -> Unit)?,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val columns = herd.groups().chunkedColumns()
    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        Row(
            Modifier.fillMaxWidth().padding(start = 18.dp, top = 10.dp, end = 18.dp, bottom = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            KText("Herd", tokens.type.paneTitle, tokens.color.text)
            NodeCountPill(herd.nodes.count { it.online }, compact = true)
            NewAction(target = LANDSCAPE_TOUCH)
            Box(Modifier.weight(1f))
            if (triage.isNotEmpty()) {
                StatusBadge(
                    if (triage.size > 1) "Needs you · ${triage.size}" else "Needs you",
                    tokens.color.blocked,
                    tokens.color.blockedBg,
                )
            }
        }
        Row(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 10.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            for (column in columns) {
                Column(Modifier.weight(1f)) {
                    if (column === columns.first() && triage.isNotEmpty()) {
                        Box(Modifier.padding(horizontal = 6.dp, vertical = 4.dp)) {
                            TriageList(triage, compact = true, onOpen = onOpenPane, onApprove = null)
                        }
                    }
                    for (group in column) {
                        NodeHeader(group.node, localRtt, PaddingValues(start = 8.dp, end = 8.dp, top = 10.dp, bottom = 6.dp))
                        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
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

private fun List<NodeGroup>.chunkedColumns(): List<List<NodeGroup>> {
    val left = mutableListOf<NodeGroup>()
    val right = mutableListOf<NodeGroup>()
    var leftWeight = 0
    var rightWeight = 0
    for (group in this) {
        val weight = group.panes.size + 1
        if (leftWeight <= rightWeight) {
            left += group
            leftWeight += weight
        } else {
            right += group
            rightWeight += weight
        }
    }
    return listOf(left, right)
}

@Composable
fun HerdSidebar(
    herd: Herd,
    now: Double,
    localRtt: Double?,
    triage: List<TriageItem>,
    activePaneId: String?,
    deviceName: String,
    deviceDetail: String,
    onOpenPane: (String) -> Unit,
    onSettings: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    Column(
        modifier
            .width(296.dp)
            .fillMaxHeight()
            .background(tokens.color.bar)
            .edgeEnd(),
    ) {
        Row(
            Modifier.fillMaxWidth().padding(start = 18.dp, top = 18.dp, end = 18.dp, bottom = 14.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            KText("Kampr", tokens.type.screenTitle, tokens.color.text)
            Row(horizontalArrangement = Arrangement.spacedBy(9.dp), verticalAlignment = Alignment.CenterVertically) {
                NodeCountPill(herd.nodes.count { it.online }, compact = true)
                NewAction()
            }
        }
        if (triage.isNotEmpty()) {
            Box(Modifier.padding(start = 14.dp, end = 14.dp, bottom = 14.dp)) {
                TriageList(triage, compact = true, onOpen = onOpenPane, onApprove = null)
            }
        }
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            herd.groups().forEachIndexed { index, group ->
                NodeHeader(
                    group.node,
                    localRtt,
                    PaddingValues(start = 18.dp, end = 18.dp, top = if (index == 0) 0.dp else 16.dp, bottom = 7.dp),
                )
                Column(Modifier.padding(horizontal = 10.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    for (pane in group.panes) {
                        PaneRow(pane, now, pane.id == activePaneId) { onOpenPane(pane.id) }
                    }
                }
            }
        }
        Row(
            Modifier
                .fillMaxWidth()
                .edgeTop()
                .padding(start = 14.dp, top = 12.dp, end = 14.dp, bottom = 14.dp)
                .clickable(onClick = onSettings),
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
}
