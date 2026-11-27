package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.Answering
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.PendingField
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Task
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

@OptIn(ExperimentalTestApi::class)
internal fun ComposeUiTest.seamScreen(size: Pair<Dp, Dp>, content: @Composable () -> Unit) {
    setContent {
        CompositionLocalProvider(
            LocalTokens provides tokensFor(SoftTheme, if (size.first > size.second) TypeScale.Phone else TypeScale.Phone),
            LocalPaneIo provides RecordingIo,
            LocalConnectionStatus provides ConnectionStatus.Live("full"),
        ) {
            Box(Modifier.size(size.first, size.second)) { content() }
        }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
internal fun ComposeUiTest.seamView(size: Pair<Dp, Dp>, pane: PaneState, info: dev.kampr.shared.wire.PaneInfo = demoInfo(status = "idle", converses = true)) = seamScreen(size) { ConversationView(pane, info, Modifier.fillMaxSize(), clock = { SEAM_NOW }) }

@OptIn(ExperimentalTestApi::class)
private fun SemanticsNodeInteraction.shown(): Rect = fetchSemanticsNode().boundsInRoot

@OptIn(ExperimentalTestApi::class)
private fun SemanticsNodeInteraction.layout(): TextLayoutResult {
    val results = mutableListOf<TextLayoutResult>()
    fetchSemanticsNode().config[SemanticsActions.GetTextLayoutResult].action?.invoke(results)
    return results.single()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.panel(): Rect = onAllNodesWithContentDescription("Explore — Map the manage op end-to-end path, agent", substring = true)
    .fetchSemanticsNodes()
    .maxBy { it.boundsInRoot.height }
    .boundsInRoot

private val TWELVE_TASKS = (1..12).map { Task("x$it", "shell", call = "make target-$it", since = "2026-11-27T11:5$it:00Z".take(20)) }

@OptIn(ExperimentalTestApi::class)
class PendingBudgetTest {
    // tc-plan-land.png: the plan grew the card past the window and its answers were drawn below
    // it, out of reach. The answers are measured before the plan, and the plan says it goes on.
    @Test
    fun aLongPlanOnARotatedPhoneKeepsItsAnswersOnScreen() = runComposeUiTest {
        seamView(LANDSCAPE, seamPane(PLAN_ASK))
        val root = onRoot().shown()
        val first = onNodeWithContentDescription("Answer 1, Yes, and auto-accept edits").shown()
        assertTrue(first.height >= 36f * density.density && first.bottom <= root.bottom, "the first answer is cut or off screen: $first in $root")
        onNodeWithText("more below ↓").fetchSemanticsNode()
        val card = onNodeWithContentDescription("The agent is asking:", substring = true).shown()
        assertTrue(card.height <= root.height * 0.66f, "the card took ${card.height} of ${root.height}")
    }

    // tc-tasks-and-question-land.png: the open task panel was drawn over the question's options.
    // A question folds the panel, and the two share one budget so the transcript keeps a third.
    @Test
    fun aQuestionFoldsTheTasksAndTheTwoLeaveTheTranscriptRoom() = runComposeUiTest {
        val store = seamStore(ServerMsg.ConvoTasks(PANE_ID, SEAM_TASKS))
        seamView(LANDSCAPE, store.pane(PANE_ID))
        onNodeWithContentDescription("Show the tasks").performClick()
        waitForIdle()
        assertEquals(1, onAllNodesWithContentDescription("Open npm run dev", substring = true).fetchSemanticsNodes().size)
        store.accept(INDENT_ASK)
        waitForIdle()
        assertEquals(0, onAllNodesWithContentDescription("Open npm run dev", substring = true).fetchSemanticsNodes().size, "the panel stayed open over the question")
        val option = onNodeWithContentDescription("Answer 1, Tabs").shown()
        val panel = panel()
        assertFalse(option.overlaps(panel), "the panel $panel covers the answer $option")
        val card = onNodeWithContentDescription("The agent is asking:", substring = true).shown()
        assertTrue(card.height + panel.height <= onRoot().shown().height * 0.66f, "card ${card.height} + panel ${panel.height}")
    }

    // hl-long-everything-phone.png: an open panel of tasks took half the screen.
    @Test
    fun anOpenTaskPanelScrollsInsideItsShareOfThePane() = runComposeUiTest {
        seamView(PORTRAIT, seamPane(ServerMsg.ConvoTasks(PANE_ID, TWELVE_TASKS), turns = 20))
        onNodeWithContentDescription("Show the tasks").performClick()
        waitForIdle()
        val panel = onAllNodesWithContentDescription("make target-1, shell", substring = true).fetchSemanticsNodes().maxBy { it.boundsInRoot.height }.boundsInRoot
        assertTrue(panel.height <= onRoot().shown().height * 0.41f, "the panel is ${panel.height} of ${onRoot().shown().height}")
    }

    // A "don't ask again" option carries its scope at the end of the label, past one line.
    @Test
    fun aLongAnswerLabelWrapsRatherThanLosingItsScope() = runComposeUiTest {
        val label = "Yes, and don't ask again for git commands in /home/dbrain/dev/kampr/crates"
        val ask = INDENT_ASK.copy(options = listOf(PendingOption("1", "Yes"), PendingOption("2", label)))
        seamScreen(PORTRAIT) { PendingStrip(ask, Answering.Ready, {}, Modifier.fillMaxWidth()) }
        assertFalse(onNodeWithText("2 · $label", useUnmergedTree = true).layout().hasVisualOverflow, "the label is cut")
    }

    // hl-form-phone.png: the fields were dumped in monospace above the form that draws them, and
    // a field's "optional" lived in a placeholder cut to the box and gone at the first key.
    @Test
    fun anOpenFormDrawsItsFieldsOnceWithLabelsThatStay() = runComposeUiTest {
        val form = ServerMsg.Pending(
            pane = PANE_ID,
            question = "The deploy server needs some details",
            header = "Deploy",
            kind = "form",
            options = listOf(PendingOption("1", "Fill in"), PendingOption("2", "Decline")),
            fields = listOf(PendingField("replicas", "Replicas", type = "integer"), PendingField("note", "Release note", detail = "Shown in the changelog")),
            source = "conversation",
        )
        seamScreen(PORTRAIT) { PendingStrip(form, Answering.Ready, {}, Modifier.fillMaxWidth(), onForm = {}) }
        onNodeWithContentDescription("Answer 1, Fill in").performClick()
        waitForIdle()
        assertEquals(0, onAllNodesWithText("Replicas (replicas)", substring = true).fetchSemanticsNodes().size, "the schema dump is still drawn")
        onNodeWithContentDescription("Form field: Replicas").performTextInput("3")
        waitForIdle()
        assertEquals(1, onAllNodesWithText("Replicas · optional").fetchSemanticsNodes().size, "the label went with the first key")
    }

    // A plan short enough to show whole says nothing about more.
    @Test
    fun aPlanThatFitsHasNoMoreCue() = runComposeUiTest {
        seamView(PORTRAIT, seamPane(PLAN_ASK.copy(context = "- one step")))
        assertEquals(0, onAllNodesWithText("more below ↓").fetchSemanticsNodes().size)
    }
}
