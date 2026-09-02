package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertTrue

private const val PANE_ID = "01JNODE/w1:p1"

// A herdr pane as the node actually reports one, with an agent in it. `kampr · claude` is fourteen
// characters — the ordinary case, not a pathological name.
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

// 1080x2400 at 480 dpi is 360 x 800 dp — the profile the report came off. 411 x 914 is the same
// panel at 420 dpi, and the landscape figure is the first rotated.
private val PHONE_480 = 360.dp to 800.dp
private val PHONE_420 = 411.dp to 914.dp
private val LANDSCAPE_480 = 800.dp to 360.dp



private class Header(
    val name: String,
    val size: Pair<Dp, Dp>,
    val info: PaneInfo = INFO,
    val readOnly: Boolean = false,
    val stale: Boolean = false,
    val unsent: Int = 0,
    val view: PaneView = PaneView.Terminal,
)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.header(case: Header) {
    val (width, height) = case.size
    setContent {
        Bars {
            CompositionLocalProvider(LocalManage provides AllowManage) {
                Box(Modifier.size(width, height)) {
                    PaneScreenMobile(
                        pane = PaneState(PANE_ID, StyleTable()).also {
                            it.stale = case.stale
                            repeat(case.unsent) { _ -> it.noteUndelivered() }
                        },
                        info = case.info,
                        view = case.view,
                        surfaces = BlankSurfaces,
                        landscape = width > height,
                        readOnly = case.readOnly,
                        onBack = {},
                        onView = {},
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
    }
    waitForIdle()
}

// What the glyphs actually need against what the layout handed the node. `hasVisualOverflow` sits
// exactly on the boundary when the two are equal, so the comparison carries a pixel of slack and
// says the numbers when it fails.
@OptIn(ExperimentalTestApi::class)
private fun SemanticsNodeInteraction.assertNotCut(where: String) {
    val node = fetchSemanticsNode()
    val out = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
    node.config[SemanticsActions.GetTextLayoutResult].action!!.invoke(out)
    val wanted = out.first().multiParagraph.maxIntrinsicWidth
    val given = node.boundsInRoot.width
    assertTrue(
        wanted <= given + 1f,
        "$where: the title needs ${wanted}px and the header gave it ${given}px, so it is cut off",
    )
}

// A 1080x2400 phone at 480 dpi is 360 dp wide. The header packs a back target, two 44 dp actions
// and up to four badges around the one elastic thing on the line, so the title is handed whatever
// is left — 69 px of the 133 px it needs with an agent that is merely blocked, and nothing at all
// once the pane is also stale and read-only.
@OptIn(ExperimentalTestApi::class)
class PaneTitleTest {
    private val cases = listOf(
        Header("480 dpi, blocked", PHONE_480),
        Header("480 dpi, idle", PHONE_480, INFO.copy(agentStatus = "idle")),
        Header("480 dpi, idle and read-only", PHONE_480, INFO.copy(agentStatus = "idle"), readOnly = true),
        Header("480 dpi, blocked and stale and read-only", PHONE_480, readOnly = true, stale = true),
        Header("480 dpi, blocked and stale with keystrokes lost", PHONE_480, stale = true, unsent = 7),
        Header("420 dpi, blocked", PHONE_420),
        Header("480 dpi landscape, blocked", LANDSCAPE_480),
    )

    @Test
    fun theTitleIsNotCutOffOnAnyPhoneTheReportCoversThis() {
        for (case in cases) {
            runComposeUiTest {
                header(case)
                onNodeWithText(paneTitle(case.info), useUnmergedTree = true).assertNotCut(case.name)
            }
        }
    }

    // Moving something off the title line only counts if it is still on the screen: a status that
    // vanished would read as "fixed" to a width assertion and as a regression to an operator.
    @Test
    fun everythingThatSharedTheLineIsStillThere() {
        runComposeUiTest {
            header(Header("crowded", PHONE_480, readOnly = true, stale = true))
            onNodeWithContentDescription("This agent is blocked").assertExists()
            onNodeWithContentDescription("Stale — this pane has stopped sending frames, showing the last grid").assertExists()
            onNodeWithContentDescription("This device is read-only — it cannot type into the pane").assertExists()
            onNodeWithContentDescription("New, from this pane").assertExists()
            onNodeWithContentDescription("Pane actions").assertExists()
            onNodeWithContentDescription("Back to the herd").assertExists()
            onNodeWithContentDescription("Terminal view").assertExists()
            onNodeWithContentDescription("Conversation view").assertExists()
        }
    }

    // The badge is the only thing that says the picture stopped, and it used to say it about the
    // grid whichever surface was up — so the transcript, which is the one a reader cannot date by
    // looking at it, was told it was a stale terminal.
    @Test
    fun theStaleBadgeNamesTheSurfaceTheReaderIsLookingAt() {
        for ((view, said) in listOf(
            PaneView.Terminal to "showing the last grid",
            PaneView.Conversation to "showing the last transcript that arrived",
        )) {
            runComposeUiTest {
                header(Header("stale on $view", PHONE_480, stale = true, view = view))
                onNodeWithContentDescription(
                    "Stale — this pane has stopped sending frames, $said",
                ).assertExists()
            }
        }
    }
}
