package dev.kampr.mosaic

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.groups
import dev.kampr.shared.model.othersWatching
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.model.watchersPhrase
import dev.kampr.shared.model.watchersTag
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.BottomSheet
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.Dot
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.SheetCard
import dev.kampr.shared.ui.SheetHeader
import dev.kampr.shared.ui.SheetSection
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.nodeReach
import dev.kampr.shared.ui.statusColor
import dev.kampr.shared.ui.statusIcon
import dev.kampr.shared.ui.statusWord
import dev.kampr.shared.util.formatLatency

// Grouped by node, then by session, because that is what a merged herd is: a named session is
// its own herdr server and joins as its own node, so the two levels are host and session.
@Composable
fun PanePicker(
    herd: Herd,
    breakpoint: Breakpoint,
    chosen: List<String>,
    full: Boolean,
    onPick: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val tokens = Kampr.tokens
    val compact = breakpoint != Breakpoint.Desktop
    val byHost = herd.groups().groupBy { it.node.host }
    BottomSheet(breakpoint, onDismiss) {
        SheetHeader(
            title = "Add pane",
            subtitle = if (full) "the mosaic is full — remove one first" else "${chosen.size} of $MAX_CELLS in the mosaic",
            onBack = null,
            onClose = onDismiss,
            compact = compact,
        )
        Column(
            Modifier.verticalScroll(rememberScrollState()).padding(bottom = 18.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            for ((host, sessions) in byHost) {
                SheetSection(host, compact)
                for (group in sessions) {
                    val node = group.node
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .named(
                                "${node.session} on ${node.host}, " +
                                    (if (node.online) "online" else "offline") + ", " +
                                    formatLatency(node.rttMs),
                            )
                            .padding(start = 20.dp, end = 20.dp, bottom = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Dot(if (node.online) tokens.color.done else tokens.color.blocked, 6.dp)
                        LabelText(node.session, tokens.type.micro, tokens.color.dim)
                        KText(
                            listOfNotNull(
                                nodeReach(node),
                                formatLatency(node.rttMs),
                                node.build,
                            ).joinToString(" · "),
                            tokens.type.meta,
                            tokens.color.mute,
                            Modifier.weight(1f),
                        )
                    }
                    if (group.panes.isEmpty()) {
                        Box(Modifier.padding(start = 20.dp, end = 20.dp, bottom = 6.dp)) {
                            KText(
                                node.detail ?: "no panes",
                                tokens.type.captionSmall,
                                tokens.color.mute,
                            )
                        }
                    }
                    Column(
                        Modifier.padding(horizontal = 16.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        for (pane in group.panes) {
                            val already = pane.id in chosen
                            val status = statusOf(pane)
                            SheetCard(
                                icon = statusIcon(status),
                                iconTint = statusColor(status),
                                title = paneTitle(pane),
                                subtitle = listOfNotNull(
                                    pane.cwd,
                                    "${pane.cols?.toString() ?: "—"}×${pane.rows}",
                                    watchersTag(othersWatching(pane)),
                                ).joinToString(" · "),
                                subtitleMono = true,
                                selected = already,
                                compact = compact,
                                onClick = if (already || full) null else ({ onPick(pane.id) }),
                                label = listOfNotNull(
                                    "Add ${paneTitle(pane)}",
                                    statusWord(status),
                                    watchersPhrase(othersWatching(pane)),
                                    "to the mosaic",
                                ).joinToString(", "),
                                trailing = if (already) ({
                                    KText("in the mosaic", tokens.type.micro, tokens.color.accent)
                                }) else null,
                            )
                        }
                    }
                }
            }
        }
    }
}
