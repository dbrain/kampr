package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.PaneCard
import dev.kampr.shared.ui.PaneRow
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ProgramRow
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JNODE/n:01ja"

private object Bare : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)

    @Composable override fun Zoom(pane: PaneState, modifier: Modifier) = Unit
}

private fun pane(
    status: String = "idle",
    error: Boolean = false,
    progress: Int? = null,
    records: List<ProgramRow>? = null,
) = PaneInfo(
    id = PANE,
    nodeId = "01JNODE",
    provider = "native",
    label = "build",
    cwd = "/home/u/dev",
    agentStatus = status,
    programError = error,
    programMsg = "boom".takeIf { error },
    programProgress = progress,
    program = records,
)

private val RECORDS = listOf(
    ProgramRow("test", "working", progress = 40, app = "ci"),
    ProgramRow("lint/slow", "error", app = "ci", title = "Slow rules", msg = "3 failed"),
)

private enum class Chrome { Portrait, Landscape, Desktop }

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.header(info: PaneInfo, layout: Chrome) = setContent {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalManage provides AllowManage) {
        Box(Modifier.fillMaxSize()) {
            val state = PaneState(PANE, StyleTable())
            when (layout) {
                Chrome.Desktop -> PaneScreenDesktop(state, info, PaneView.Terminal, Bare, false, onView = {})
                else -> PaneScreenMobile(state, info, PaneView.Terminal, Bare, layout == Chrome.Landscape, false, onBack = {}, onView = {})
            }
        }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.rows(info: PaneInfo) = setContent {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalManage provides AllowManage) {
        androidx.compose.foundation.layout.Column {
            PaneCard(info, 0.0, {})
            PaneRow(info, 0.0, false) {}
        }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.count(label: String): Int = onAllNodesWithContentDescription(label, substring = true).fetchSemanticsNodes().size

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.progress(): List<Float> = onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.ProgressBarRangeInfo))
    .fetchSemanticsNodes()
    .mapNotNull { it.config.getOrNull(SemanticsProperties.ProgressBarRangeInfo)?.current }

// docs/24's client half: what a pane's programs reported over OSC 7501 drawn with the marks the herd
// already has. `error` is a state `agent_status` cannot say, so it rides beside an idle ring.
@OptIn(ExperimentalTestApi::class)
class ProgramStatusUiTest {
    @Test
    fun aFailedProgramIsMarkedOnBothHerdRowsAndSaysWhy() = runComposeUiTest {
        rows(pane(error = true))
        assertEquals(2, count("Idle, Failed: boom"), "each row says it failed, and the program's words")
        assertEquals(4, onAllNodesWithContentDescription("Failed: boom", substring = true, useUnmergedTree = true).fetchSemanticsNodes().size, "each row's mark carries the words too")
    }

    @Test
    fun aRowWithNoErrorSaysNothingOfOne() = runComposeUiTest {
        rows(pane())
        assertEquals(0, count("Failed"))
    }

    @Test
    fun theHeaderMarksTheFailureOnEveryLayout() {
        for (layout in Chrome.entries) {
            runComposeUiTest {
                header(pane(error = true), layout)
                assertTrue(count("The program failed: boom") >= 1, "$layout")
            }
            runComposeUiTest {
                header(pane(), layout)
                assertEquals(0, count("The program failed"), "$layout")
            }
        }
    }

    @Test
    fun aWorkingRowAndHeaderCarryTheProgressTheyWereGiven() {
        runComposeUiTest {
            rows(pane(status = "working", progress = 40))
            assertEquals(listOf(0.4f, 0.4f), progress())
        }
        runComposeUiTest {
            rows(pane(status = "working"))
            assertEquals(emptyList(), progress())
        }
        for (layout in Chrome.entries) {
            runComposeUiTest {
                header(pane(status = "working", progress = 40), layout)
                assertEquals(listOf(0.4f), progress(), "$layout")
            }
        }
    }

    @Test
    fun theRecordsAreListedFromTheStatusOnADesk() = runComposeUiTest {
        header(pane(status = "working", records = RECORDS), Chrome.Desktop)
        assertEquals(0, count("ci · Slow rules, error, 3 failed"))
        onNodeWithContentDescription("2 programs", substring = true).performClick()
        assertEquals(1, count("ci, working, 40%"))
        assertEquals(1, count("ci · Slow rules, error, 3 failed"))
    }

    @Test
    fun anUprightPhoneListsThemFromALongPressOnItsMark() = runComposeUiTest {
        header(pane(status = "working", records = RECORDS), Chrome.Portrait)
        onNodeWithContentDescription("This agent is working", substring = true).performTouchInput { longClick() }
        waitForIdle()
        assertEquals(1, count("ci, working, 40%"))
    }

    @Test
    fun aPaneWithOnlyARootOffersNoList() = runComposeUiTest {
        header(pane(status = "working"), Chrome.Desktop)
        assertEquals(0, count("programs"))
    }
}
