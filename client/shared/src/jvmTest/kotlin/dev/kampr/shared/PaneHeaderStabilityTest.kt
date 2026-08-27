package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SkikoComposeUiTest
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.v2.runSkikoComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.named
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE_ID = "01JNODE/w1:p1"

private const val ZOOM = "Zoom probe"

private val INFO = PaneInfo(
    id = PANE_ID,
    nodeId = "01JNODE",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = "blocked",
    cols = 94,
    rows = 40,
    hasConversation = true,
)

// The real control is 68 dp wide before its label and 44 dp tall, and the header measures whatever
// it is handed — a probe narrower than that would reserve a slot the button does not fit in.
private object ZoomProbe : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)
    @Composable override fun Zoom(pane: PaneState, modifier: Modifier) =
        Box(modifier.size(71.dp, 44.dp).named(ZOOM))
}

// The fonts Kampr ships, not the machine's. `FontFamily.Default` and `.Monospace` resolve through
// the system font manager, so this measured a header nobody has and measured a different one on
// every machine: it passed on every developer's box and failed on a bare CI runner for five
// releases running, over a real defect it could only see where the metrics were wide enough to
// force a wrap. A harness that is not the app is not evidence either way (#191).
//
// What that defect was is now asserted directly, and against these same fonts, by
// `SegmentedWidthTest` — a control that asks for a different width depending on which segment is
// selected. This board stays as the other end of it: the header composed whole.
private fun tokens() = tokensFor(themeOf("soft"), TypeScale.Phone, Ground.Dark)

private class Board(
    val name: String,
    val width: Dp,
    val height: Dp,
    val desktop: Boolean = false,
    val landscape: Boolean = false,
    val crowded: Boolean = false,
)

// Every layout that carries both a zoom control and a view switcher, and each of the two phone ones
// again at 360 dp with the badges the header was already known to run out of room for — stale and
// read-only against a blocked agent is what makes the second row wrap. `runComposeUiTest` clamps
// its content to 1024 dp, which two of these are wider than, so the size has to be the scene's.
private val BOARDS = listOf(
    Board("portrait", 411.dp, 914.dp),
    Board("portrait crowded", 360.dp, 800.dp, crowded = true),
    Board("landscape", 914.dp, 411.dp, landscape = true),
    Board("landscape crowded", 740.dp, 360.dp, landscape = true, crowded = true),
    Board("desktop", 1280.dp, 860.dp, desktop = true),
    Board("desktop crowded", 1280.dp, 860.dp, desktop = true, crowded = true),
)

@OptIn(ExperimentalTestApi::class)
private fun SkikoComposeUiTest.header(board: Board, view: MutableState<PaneView>) {
    setContent {
        CompositionLocalProvider(LocalTokens provides tokens(), LocalManage provides AllowManage) {
            Box(Modifier.fillMaxSize()) {
                if (board.desktop) {
                    PaneScreenDesktop(
                        pane = PaneState(PANE_ID, StyleTable()).also { it.stale = board.crowded },
                        info = INFO,
                        view = view.value,
                        surfaces = ZoomProbe,
                        readOnly = board.crowded,
                        onView = {},
                        onAnswer = {},
                        modifier = Modifier.fillMaxSize(),
                    )
                } else {
                    PaneScreenMobile(
                        pane = PaneState(PANE_ID, StyleTable()).also { it.stale = board.crowded },
                        info = INFO,
                        view = view.value,
                        surfaces = ZoomProbe,
                        landscape = board.landscape,
                        readOnly = board.crowded,
                        onBack = {},
                        onView = {},
                        onAnswer = {},
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
    }
    waitForIdle()
}

// Everything the header describes to a reader and where it sits — the two view segments, the glyph
// actions, and the badges beside them — rather than a list of the ones somebody thought to name.
// Described nodes and not every text node: the selected segment's label is a heavier weight than
// the others, so the glyph run inside a tab moves a pixel or two on selection whatever the header
// does. That is the selection, not a reflow. The surfaces under the chrome are blank probes, so
// nothing outside the header is described at all.
private fun SemanticsNode.collect(into: MutableList<Pair<String, Rect>>) {
    val label = config.getOrNull(SemanticsProperties.ContentDescription)?.firstOrNull()
    if (label != null && label != ZOOM) into += label to boundsInRoot
    children.forEach { it.collect(into) }
}

@OptIn(ExperimentalTestApi::class)
private fun SkikoComposeUiTest.headerBounds(): List<Pair<String, Rect>> =
    mutableListOf<Pair<String, Rect>>().also { onRoot(useUnmergedTree = true).fetchSemanticsNode().collect(it) }

@OptIn(ExperimentalTestApi::class)
private fun SkikoComposeUiTest.zooms(): Int =
    onAllNodesWithContentDescription(ZOOM).fetchSemanticsNodes().size

// The report: "it hides in conversation mode then shows again in terminal mode and makes the view
// switcher jump around". The zoom sheet is the terminal surface's, so the control that opens it has
// nothing to open on a transcript — but a header that drops an item reflows everything elastic
// beside it, and the segment the thumb is resting on slides out from under it.
@OptIn(ExperimentalTestApi::class)
class PaneHeaderStabilityTest {
    @Test
    fun nothingInThePaneHeaderMovesWhenTheZoomControlLeavesWithTheTerminal() {
        val moved = BOARDS.flatMap { board ->
            var terminal: List<Pair<String, Rect>> = emptyList()
            var conversation: List<Pair<String, Rect>> = emptyList()
            var offered = 0
            var withdrawn = -1
            runSkikoComposeUiTest(Size(board.width.value, board.height.value), Density(1f)) {
                val view = mutableStateOf(PaneView.Terminal)
                header(board, view)
                terminal = headerBounds()
                offered = zooms()
                view.value = PaneView.Conversation
                waitForIdle()
                conversation = headerBounds()
                withdrawn = zooms()
            }
            // A header that simply kept the control would hold everything still and still fail the
            // report: the sheet it opens is drawn by a surface the transcript replaced.
            assertEquals(1, offered, "${board.name}: the terminal view offered no zoom control")
            assertEquals(0, withdrawn, "${board.name}: a zoom control survived onto the transcript")
            assertTrue(
                terminal.map { it.first }.containsAll(listOf("Terminal view", "Conversation view")),
                "${board.name}: measured a header with no view switcher in it",
            )
            terminal.zip(conversation)
                .filter { (was, now) -> was != now }
                .map { (was, now) -> "${board.name}: $was became $now" }
                .ifEmpty {
                    if (terminal.size == conversation.size) emptyList()
                    else listOf("${board.name}: the header lost ${terminal.map { it.first } - conversation.map { it.first }.toSet()}")
                }
        }
        assertEquals(emptyList(), moved, "the pane header reflowed when the pane changed view")
    }
}
