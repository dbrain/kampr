package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals

private fun pane(id: String, label: String) = PaneInfo(id = "01JNODE/$id", nodeId = "01JNODE", label = label, provider = "native")

private val TAB = listOf(pane("n:a", "server"), pane("n:b", "tests"), pane("n:c", "logs"))

// A phone has room for one pane, so a tab of three is flattened to a strip of three: the pane on
// screen is marked, and a tap goes to its neighbour rather than squeezing three grids into one.
@OptIn(ExperimentalTestApi::class)
class PaneTabStripTest {
    @Test
    fun aPhoneShowsAPaneInATabWithAStripOfItsNeighbours() = runComposeUiTest {
        val opened = mutableListOf<String>()
        val shown = TAB[1]
        setContent {
            Bars {
                Box(Modifier.size(412.dp, 900.dp)) {
                    PaneScreenMobile(
                        pane = PaneState(shown.id, StyleTable()),
                        info = shown,
                        view = PaneView.Terminal,
                        surfaces = BlankSurfaces,
                        landscape = false,
                        readOnly = false,
                        onBack = {},
                        onView = {},
                        tab = TAB,
                        onTab = { opened += it },
                    )
                }
            }
        }
        waitForIdle()
        onAllNodesWithContentDescription("in this tab", substring = true).assertCountEquals(3)
        onNodeWithContentDescription("${paneTitle(TAB[1])}, in this tab").assertIsSelected()
        onNodeWithContentDescription("${paneTitle(TAB[2])}, in this tab").performClick()
        waitForIdle()
        assertEquals(listOf(TAB[2].id), opened)
    }

    @Test
    fun aPaneAloneInItsTabHasNoStrip() = runComposeUiTest {
        setContent {
            Bars {
                Box(Modifier.size(412.dp, 900.dp)) {
                    PaneScreenMobile(
                        pane = PaneState(TAB[0].id, StyleTable()),
                        info = TAB[0],
                        view = PaneView.Terminal,
                        surfaces = BlankSurfaces,
                        landscape = false,
                        readOnly = false,
                        onBack = {},
                        onView = {},
                        tab = TAB.take(1),
                    )
                }
            }
        }
        waitForIdle()
        onAllNodesWithContentDescription("in this tab", substring = true).assertCountEquals(0)
    }

    // A desk has the room to show the tab whole, so its strip carries the way to open it side by side.
    @Test
    fun aDeskOffersTheWholeTabSideBySide() = runComposeUiTest {
        var sideBySide = 0
        setContent {
            Bars {
                Box(Modifier.size(1400.dp, 900.dp)) {
                    PaneScreenDesktop(
                        pane = PaneState(TAB[0].id, StyleTable()),
                        info = TAB[0],
                        view = PaneView.Terminal,
                        surfaces = BlankSurfaces,
                        readOnly = false,
                        onView = {},
                        tab = TAB,
                        onSideBySide = { sideBySide++ },
                    )
                }
            }
        }
        waitForIdle()
        onNodeWithContentDescription("Show this tab's 3 panes side by side").performClick()
        assertEquals(1, sideBySide)
    }
}
