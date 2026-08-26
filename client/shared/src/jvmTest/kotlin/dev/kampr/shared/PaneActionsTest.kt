package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.PaneActionsSheet
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val PANE = PaneInfo(
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

private const val FILL = "fill the tab"

@OptIn(ExperimentalTestApi::class)
@Composable
private fun Sheet(sent: MutableList<ManageOp>) {
    CompositionLocalProvider(LocalTokens provides phoneTokens()) {
        Box(Modifier.fillMaxSize()) {
            PaneActionsSheet(Breakpoint.Portrait, PANE, null, { sent += it }, {})
        }
    }
}

// The operator, verbatim: "i dont even get what zoom at the desk is aha".
//
// It is herdr's `pane.zoom` — the pane fills its tab on the machine and its siblings are hidden —
// and it works (probe #265). What it is not is Kampr's own zoom, which is the magnification of the
// rendered grid and lives one screen away in the pane header, announced as "Zoom, currently 1.6x".
// Two unrelated controls with one name, and the sheet's "at the desk" was the whole of the
// disambiguation. This test is about the name, because the name was the defect.
@OptIn(ExperimentalTestApi::class)
class PaneActionsTest {
    @Test
    fun theActionsSheetNeverOffersASecondControlCalledZoom() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent { Sheet(sent) }
        waitForIdle()
        val spoken = onAllNodesWithContentDescription("zoom", substring = true, ignoreCase = true)
            .fetchSemanticsNodes()
        val written = onAllNodesWithText("zoom", substring = true, ignoreCase = true)
            .fetchSemanticsNodes()
        assertEquals(
            0,
            spoken.size + written.size,
            "the actions sheet says \"zoom\" ${spoken.size + written.size} times, " +
                "and the only zoom this app has is the one in the pane header",
        )
    }

    @Test
    fun herdrsZoomIsStillReachableUnderANameThatSaysWhatItDoes() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent { Sheet(sent) }
        waitForIdle()
        onNodeWithText(FILL).performClick()
        assertTrue(
            sent.any { it is ManageOp.PaneZoom },
            "clicking \"$FILL\" sent $sent, not the pane.zoom it is a name for",
        )
    }
}
