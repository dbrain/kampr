package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals

private val TALKING = PaneInfo(
    id = "01JNODE/w1:p1",
    nodeId = "01JNODE",
    workspace = "kampr",
    cwd = "/home/u/kampr",
    agent = "claude",
    agentStatus = "idle",
    cols = 94,
    rows = 40,
    hasConversation = true,
)

private class TopOwed : PaneSurfaces {
    var top: Dp? = null

    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) {
        top = LocalSafeArea.current.top
        Box(modifier)
    }

    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)
}

// The pane header pays for the status bar and the conversation is laid out below it, so anything
// in the conversation that reached the window's top — the file and picture viewers — paid it a
// second time and opened under a bar-high strip of nothing.
@OptIn(ExperimentalTestApi::class)
class ConversationTopEdgeTest {
    private fun owed(screen: @Composable (PaneSurfaces) -> Unit): Dp? {
        val surfaces = TopOwed()
        runComposeUiTest {
            setContent { Bars { screen(surfaces) } }
            waitForIdle()
        }
        return surfaces.top
    }

    @Test
    fun theConversationUnderThePaneHeaderOwesTheStatusBarNothing() {
        for (landscape in listOf(false, true)) {
            val top = owed { surfaces ->
                PaneScreenMobile(
                    pane = PaneState("01JNODE/w1:p1", StyleTable()),
                    info = TALKING,
                    view = PaneView.Conversation,
                    surfaces = surfaces,
                    landscape = landscape,
                    readOnly = false,
                    onBack = {},
                    onView = {},
                    modifier = Modifier.fillMaxSize(),
                )
            }
            assertEquals(0.dp, top, "landscape=$landscape")
        }
        val desk = owed { surfaces ->
            PaneScreenDesktop(
                pane = PaneState("01JNODE/w1:p1", StyleTable()),
                info = TALKING,
                view = PaneView.Conversation,
                surfaces = surfaces,
                readOnly = false,
                onView = {},
                modifier = Modifier.fillMaxSize(),
            )
        }
        assertEquals(0.dp, desk, "desk")
    }
}
