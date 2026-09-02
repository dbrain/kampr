package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.BottomSheet
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.Chip
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.PaneActionsSheet
import dev.kampr.shared.ui.PrimaryAction
import dev.kampr.shared.ui.SheetCard
import dev.kampr.shared.ui.SheetHeader
import dev.kampr.shared.wire.PaneInfo

internal val SHEET_PANE = PaneInfo(
    id = "01JNODE/w3:p2",
    nodeId = "01JNODE",
    workspaceId = "01JNODE/w3",
    tabId = "01JNODE/w3:t1",
    workspace = "kampr",
    tab = "1",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = "blocked",
    cols = 74,
    rows = 30,
)

internal const val SHEET_PROSE = "the pane is on front and it has been blocked for a while"

@Composable
internal fun Actions() {
    CompositionLocalProvider(LocalTokens provides phoneTokens()) {
        Box(Modifier.fillMaxSize()) { PaneActionsSheet(Breakpoint.Portrait, SHEET_PANE, null, {}, {}) }
    }
}

// One sheet carrying both halves of the rule: a chip and a disabled button, whose words are the
// controls themselves; and a line of prose and a card, whose words are what the sheet is about.
@Composable
internal fun Mixed() {
    Bars {
        Box(Modifier.fillMaxSize()) {
            BottomSheet(Breakpoint.Portrait, {}) {
                SheetHeader("Details", null, null, {})
                KText(SHEET_PROSE, Kampr.tokens.type.body, Kampr.tokens.color.text)
                Chip("rename", selected = false, onClick = {})
                SheetCard(null, null, "front", "this machine", onClick = null)
                PrimaryAction("Send", {}, enabled = false)
            }
        }
    }
}
