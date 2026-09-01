package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.HerdSidebar
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LocalFleet
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.LocalMosaic
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.RAIL_WIDTH
import dev.kampr.shared.ui.SIDEBAR_WIDTH
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private object Manageable : ManageIo {
    override val enabled = true
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
}

private val SIDEBAR_HERD = Herd(
    nodes = listOf(
        NodeInfo("01JNODE", "steppe", kind = "local", online = true),
        NodeInfo("01JOTHER", "coming-clean", kind = "peer", online = true),
    ),
    panes = listOf(
        PaneInfo("01JNODE/w1:p1", "01JNODE", "01JNODE/w1", "01JNODE/w1:t1", "kampr", "1", cwd = "/home/dbrain/dev/kampr", agent = "claude", agentStatus = "working"),
        PaneInfo("01JNODE/w1:p2", "01JNODE", "01JNODE/w1", "01JNODE/w1:t2", "notes", "2", cwd = "/home/dbrain/notes"),
        PaneInfo("01JOTHER/w2:p1", "01JOTHER", "01JOTHER/w2", "01JOTHER/w2:t1", "herdr", "1", cwd = "/home/dbrain/dev/herdr", agent = "claude", agentStatus = "blocked"),
    ),
)

// Every control the header carries, by the name a reader reaches it under.
private val HEADER_CONTROLS = listOf(
    "Machines",
    "Mosaic, several panes at once",
    "Fleet, one command across the herd",
    "New workspace or session",
)

// The one thing in the sidebar that spans its whole width, so its bounds are the sidebar's own.
private const val FOOT = "Settings —"

@OptIn(ExperimentalTestApi::class)
@Composable
private fun Sidebar(
    collapsed: Boolean = false,
    onCollapsed: (Boolean) -> Unit = {},
    activePaneId: String? = null,
    onOpenPane: (String) -> Unit = {},
) {
    CompositionLocalProvider(
        LocalTokens provides phoneTokens(),
        LocalManage provides Manageable,
        LocalMosaic provides {},
        LocalFleet provides {},
    ) {
        // Wider than the sidebar on purpose: a header that overflows is clipped by the sidebar and
        // so is invisible in a window sized to it, which is how this shipped.
        Box(Modifier.size(SIDEBAR_WIDTH + 400.dp, 900.dp)) {
            HerdSidebar(
                herd = SIDEBAR_HERD,
                connection = ConnectionStatus.Live("full"),
                now = 0.0,
                localRtt = 12.0,
                triage = emptyList(),
                activePaneId = activePaneId,
                deviceName = "this device",
                deviceDetail = "full access",
                onOpenPane = onOpenPane,
                onSettings = {},
                collapsed = collapsed,
                onCollapsed = onCollapsed,
            )
        }
    }
}

// The report was "the + button in the sidebar is squished". It was not squished, it was gone: the
// header row overflowed 296 dp by ~44, and `GlyphAction`'s `Modifier.size` coerces to whatever
// constraint it is handed — so the last child absorbed the whole overflow and measured 0 x 0, with
// Fleet beside it losing a further dp. A control that measures nothing is not a control.
@OptIn(ExperimentalTestApi::class)
class SidebarRoomTest {
    @Test
    fun everyControlInTheSidebarHeaderIsStillAControlAtTheWidthTheSidebarHas() = runComposeUiTest {
        setContent { Sidebar() }
        waitForIdle()
        val squished = mutableListOf<String>()
        for (label in HEADER_CONTROLS) {
            val found = onAllNodesWithContentDescription(label, substring = true).fetchSemanticsNodes()
            assertTrue(found.isNotEmpty(), "the sidebar header has no \"$label\" at all")
            val box = found.first().boundsInRoot
            val width = with(density) { box.width.toDp() }
            val height = with(density) { box.height.toDp() }
            if (width < LANDSCAPE_TOUCH || height < LANDSCAPE_TOUCH) squished += "$label ${width}x$height"
        }
        assertEquals(
            emptyList(), squished,
            "the sidebar header does not fit $SIDEBAR_WIDTH and these controls absorbed it: $squished",
        )
    }

    // The overflow was only invisible because the sidebar clips it. Unclipped bounds are what say
    // whether the row fits, and they are measured against the sidebar's width, not the window's.
    @Test
    fun theSidebarHeaderDoesNotOverflowTheSidebar() = runComposeUiTest {
        setContent { Sidebar() }
        waitForIdle()
        for (label in HEADER_CONTROLS) {
            val bounds = onNodeWithContentDescription(label, substring = true).getUnclippedBoundsInRoot()
            assertTrue(
                bounds.right <= SIDEBAR_WIDTH,
                "\"$label\" ends at ${bounds.right}, past the $SIDEBAR_WIDTH the sidebar has",
            )
        }
    }

    // The operator's ask, in their words: collapsible so there's more room.
    @Test
    fun aCollapsedSidebarGivesItsRoomToWhateverIsBesideIt() = runComposeUiTest {
        var collapsed by mutableStateOf(false)
        setContent { Sidebar(collapsed = collapsed, onCollapsed = { collapsed = it }) }
        waitForIdle()
        val wide = onNodeWithContentDescription(FOOT, substring = true).getUnclippedBoundsInRoot()
        assertEquals(SIDEBAR_WIDTH, wide.right - wide.left, "the expanded sidebar is not $SIDEBAR_WIDTH wide")
        collapsed = true
        waitForIdle()
        val narrow = onNodeWithContentDescription(FOOT, substring = true).getUnclippedBoundsInRoot()
        assertEquals(RAIL_WIDTH, narrow.right - narrow.left, "collapsing the sidebar gave back no room")
    }

    // Collapsed is not hidden. Every pane the expanded sidebar listed is still reachable, still
    // says which one it is and what it is doing — that is the whole of "roughly know what you're
    // switching to".
    @Test
    fun aCollapsedSidebarStillNamesAndOpensEveryPaneItIsHiding() = runComposeUiTest {
        val opened = mutableListOf<String>()
        setContent { Sidebar(collapsed = true, onOpenPane = { opened += it }) }
        waitForIdle()
        for (pane in SIDEBAR_HERD.panes) {
            val named = onAllNodesWithContentDescription(pane.workspace!!, substring = true).fetchSemanticsNodes()
            assertTrue(named.isNotEmpty(), "the rail hides ${pane.id} without naming it")
        }
        val blocked = SIDEBAR_HERD.panes[2]
        onNodeWithContentDescription(blocked.workspace!!, substring = true).performClick()
        waitForIdle()
        assertEquals(listOf(blocked.id), opened, "a rail entry that cannot be opened is decoration")
    }

    // A pane's state is the half of "what am I switching to" that a two-letter sigil cannot carry,
    // and it is the half the operator is actually looking for.
    @Test
    fun theRailSaysWhatEachPaneIsDoingAndWhichOneIsOpen() = runComposeUiTest {
        setContent { Sidebar(collapsed = true, activePaneId = SIDEBAR_HERD.panes[1].id) }
        waitForIdle()
        onAllNodesWithContentDescription("Working", substring = true, ignoreCase = true)
            .fetchSemanticsNodes()
            .let { assertTrue(it.isNotEmpty(), "the rail never says a pane is working") }
        onAllNodesWithContentDescription("Blocked", substring = true, ignoreCase = true)
            .fetchSemanticsNodes()
            .let { assertTrue(it.isNotEmpty(), "the rail never says a pane is blocked") }
        onNodeWithContentDescription(SIDEBAR_HERD.panes[1].workspace!!, substring = true)
            .assertIsSelected()
    }

    // A one-way door is not a control.
    @Test
    fun theSidebarCollapsesAndComesBack() = runComposeUiTest {
        var collapsed by mutableStateOf(false)
        setContent { Sidebar(collapsed = collapsed, onCollapsed = { collapsed = it }) }
        waitForIdle()
        onNodeWithContentDescription("Collapse", substring = true).assertIsDisplayed().performClick()
        waitForIdle()
        assertTrue(collapsed, "the collapse control did not collapse the sidebar")
        onNodeWithContentDescription("Expand", substring = true).assertIsDisplayed().performClick()
        waitForIdle()
        assertTrue(!collapsed, "a sidebar that cannot be brought back is a sidebar the operator lost")
    }

    // The rail still has to be a way out of the herd, and the + still has to start something.
    @Test
    fun theRailKeepsTheControlsThatAreNotAPane() = runComposeUiTest {
        setContent { Sidebar(collapsed = true) }
        waitForIdle()
        onNodeWithContentDescription("New workspace or session", substring = true).assertIsDisplayed()
        onNodeWithContentDescription(FOOT, substring = true).assertIsDisplayed()
    }
}
