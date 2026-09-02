package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.PaneGone
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.model.gone
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import androidx.compose.ui.unit.Density
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val PANE = "01JNODEABCDEFGHJKMNPQRSTV/w1:p1"
private const val NODE = "01JNODEABCDEFGHJKMNPQRSTV"

private val INFO = PaneInfo(
    id = PANE,
    nodeId = NODE,
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agentStatus = "idle",
    cols = 94,
    rows = 40,
)

private const val CLOSED = "This pane has closed — what is on screen is the last thing it printed"
private const val NODE_GONE =
    "The node this pane was on has left the herd — what is on screen is the last thing it printed"

// A surfaces stub that says whether the key row was offered. Typing into a shell that has exited
// is the failure this project keeps paying for: an affordance that works perfectly and reaches
// nothing.
private class KeyRowSpy : PaneSurfaces {
    var keyRow = false
        private set

    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun Zoom(pane: PaneState, modifier: Modifier) = Box(modifier.size(40.dp))
    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) {
        keyRow = true
        Box(modifier)
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.screen(
    gone: PaneGone?,
    info: PaneInfo? = if (gone == null) INFO else null,
    surfaces: PaneSurfaces = KeyRowSpy(),
) {
    setContent {
        CompositionLocalProvider(LocalTokens provides phoneTokens()) {
            Bars {
                CompositionLocalProvider(LocalManage provides AllowManage) {
                    Box(Modifier.size(360.dp, 800.dp)) {
                        PaneScreenMobile(
                            pane = PaneState(PANE, StyleTable()),
                            info = info,
                            view = PaneView.Terminal,
                            surfaces = surfaces,
                            landscape = false,
                            readOnly = false,
                            gone = gone,
                            onBack = {},
                            onView = {},
                            modifier = Modifier.fillMaxSize(),
                        )
                    }
                }
            }
        }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
class PaneClosedTest {
    // The report: a shell exits, the pane leaves the herd, and the only thing that changed on
    // screen was the title falling back to `01JNODEABCDEFGHJKMNPQRSTV/w1:p1`. A node ULID reads as
    // the app having lost its place, not as the shell having finished.
    @Test
    fun aClosedShellSaysSoRatherThanShowingItsId() = runComposeUiTest {
        screen(PaneGone.Shell)
        onNodeWithContentDescription(CLOSED).assertExists()
        onNodeWithContentDescription("Closed — this pane is no longer in the herd").assertExists()
        assertEquals(
            0,
            onAllNodesWithText(PANE, substring = true).fetchSemanticsNodes().size,
            "the header fell back to the raw pane id, which is the whole of the report",
        )
    }

    // A pane that closed while this client was watching it still has a name, and it is the name it
    // had a moment ago — not a coordinate.
    @Test
    fun theHeaderKeepsTheNameThePaneHad() = runComposeUiTest {
        var gone by mutableStateOf<PaneGone?>(null)
        var info by mutableStateOf<PaneInfo?>(INFO)
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Bars {
                    CompositionLocalProvider(LocalManage provides AllowManage) {
                        Box(Modifier.size(360.dp, 800.dp)) {
                            PaneScreenMobile(
                                pane = PaneState(PANE, StyleTable()),
                                info = info,
                                view = PaneView.Terminal,
                                surfaces = KeyRowSpy(),
                                landscape = false,
                                readOnly = false,
                                gone = gone,
                                onBack = {},
                                onView = {},
                                modifier = Modifier.fillMaxSize(),
                            )
                        }
                    }
                }
            }
        }
        waitForIdle()
        onNodeWithText("kampr · bash").assertExists()

        info = null
        gone = PaneGone.Shell
        waitForIdle()
        onNodeWithText("kampr · bash").assertExists()
    }

    // The node leaving is not the shell exiting: one is over, the other comes back on its own.
    @Test
    fun aNodeLeavingIsItsOwnNews() = runComposeUiTest {
        screen(PaneGone.Node)
        onNodeWithContentDescription(NODE_GONE).assertExists()
        assertEquals(0, onAllNodesWithContentDescription(CLOSED).fetchSemanticsNodes().size)
    }

    // Every key on the row reaches a pane the node no longer has. Offering them is the shape this
    // codebase names outright: an affordance that looks like it works and delivers nothing.
    @Test
    fun aClosedPaneStopsOfferingTheKeys() = runComposeUiTest {
        val live = KeyRowSpy()
        screen(null, surfaces = live)
        assertTrue(live.keyRow, "the fixture never had a key row to withdraw")

        val closed = KeyRowSpy()
        screen(PaneGone.Shell, surfaces = closed)
        assertFalse(closed.keyRow, "a closed pane still offered a row of keys that reach nothing")
    }

    // The artboard the operator's eye actually lands on. Rendered rather than asserted: the claim
    // is that a closed pane looks closed from across the desk, and no semantics assertion says that.
    @Test
    fun theClosedHeaderIsAnArtboard() {
        for ((name, gone) in listOf("pane-closed" to PaneGone.Shell, "pane-node-gone" to PaneGone.Node)) {
            render(
                360.dp, 800.dp, themeOf("soft"), TypeScale.Phone,
                File("build/artboards/$name.png"), density = Density(3f),
            ) {
                Bars {
                    CompositionLocalProvider(LocalManage provides AllowManage) {
                        Box(Modifier.size(360.dp, 800.dp)) {
                            PaneScreenMobile(
                                pane = PaneState(PANE, StyleTable()),
                                info = null,
                                view = PaneView.Terminal,
                                surfaces = KeyRowSpy(),
                                landscape = false,
                                readOnly = false,
                                gone = gone,
                                onBack = {},
                                onView = {},
                                modifier = Modifier.fillMaxSize(),
                            )
                        }
                    }
                }
            }
        }
    }

    // Absence only means closed when there is a current herd to be absent from. A socket that
    // dropped, or one that has not greeted yet, must not be dressed as a shell that exited — that
    // is the reassuring lie in the other direction.
    @Test
    fun onlyACurrentHerdCanSayAPaneIsGone() {
        val node = NodeInfo(id = NODE, name = "front", kind = "local")
        assertNull(Herd().gone(PANE), "nothing has arrived yet, so nothing is missing")
        assertNull(
            Herd(nodes = listOf(node), panes = emptyList(), stale = true, known = true).gone(PANE),
            "a stale herd is the last one that arrived, not a statement about now",
        )
        assertEquals(
            PaneGone.Shell,
            Herd(nodes = listOf(node), panes = emptyList(), known = true).gone(PANE),
        )
        assertEquals(
            PaneGone.Node,
            Herd(nodes = emptyList(), panes = emptyList(), known = true).gone(PANE),
        )
        assertNull(Herd(nodes = listOf(node), panes = listOf(INFO), known = true).gone(PANE))
    }
}
