package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.getBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val CHANGE = "No, tell Claude what to change"
private const val FIELD = "Your answer: $CHANGE"
private const val SEND = "Send your answer"
private const val ASKING = "The agent is asking:"

private val PLAN_STEPS = (1..60).joinToString("\n") { "- step $it of the migration" }

private val PLAN = ServerMsg.Pending(
    pane = PANE_ID,
    id = "q9",
    header = "Plan",
    question = "Approve Claude's plan?",
    source = "hook",
    context = "## Steps\n\n$PLAN_STEPS",
    options = listOf(
        PendingOption("1", "Yes, auto-accept edits"),
        PendingOption("2", "Yes, manually approve edits"),
        PendingOption("3", CHANGE, text = true),
    ),
)

private val SUITES = ServerMsg.Pending(
    pane = PANE_ID,
    id = "q8",
    header = "Test suites",
    question = "Which test suites should I run?",
    source = "hook",
    multi = true,
    options = listOf(
        PendingOption("1", "unit", chosen = true),
        PendingOption("2", "integration"),
        PendingOption("3", "Type something", text = true),
    ),
)

private val FORM = ServerMsg.Pending(
    pane = PANE_ID,
    id = "q1",
    header = "Deploy",
    question = "Which environment should the deploy go to?",
    source = "hook",
    context = "staging-eu",
    options = emptyList(),
)

private val anAnswerChip = SemanticsMatcher("an answer chip") { node ->
    val said = if (SemanticsProperties.ContentDescription in node.config) node.config[SemanticsProperties.ContentDescription] else emptyList()
    said.any { it.startsWith("Answer ") && it.getOrNull(7)?.isDigit() == true }
}

private val INDENT = ServerMsg.Pending(
    pane = PANE_ID,
    id = "q4",
    header = "Indentation",
    question = "Which indentation do you prefer?",
    source = "hook",
    options = listOf(
        PendingOption("1", "Tabs", "Indent with tab characters, one per level, as the formatter writes them."),
        PendingOption("2", "Two spaces", "Indent with two spaces per level, which keeps deep nesting on a phone."),
        PendingOption("3", "Four spaces", "Indent with four spaces per level, as most of the codebase already does."),
        PendingOption("4", "Match the file", "Indent each file the way it is already indented, whatever that is."),
        PendingOption("5", "Type something", text = true),
    ),
)

private val DESK = 1280.dp to 860.dp

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.showing(question: ServerMsg.Pending, size: Pair<Dp, Dp> = PORTRAIT) {
    RecordingIo.sent.clear()
    val store = KamprStore()
    store.accept(ServerMsg.Convo(PANE_ID, "h-1", false, listOf(Turn("h-1", "user", null, listOf(Block.Md("plan the migration"))))))
    store.accept(question)
    setContent {
        CompositionLocalProvider(
            LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
            LocalPaneIo provides RecordingIo,
            LocalConnectionStatus provides ConnectionStatus.Live("full"),
        ) {
            Box(Modifier.size(size.first, size.second)) {
                ConversationView(store.pane(PANE_ID), demoInfo(), Modifier.fillMaxSize())
            }
        }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.fieldCount(label: String) = onAllNodesWithContentDescription(label, useUnmergedTree = true).fetchSemanticsNodes().size

// docs/12 W11: Claude's own dialogs take words as an answer, and a hook can answer them with the
// words themselves rather than a key that only moves the TUI's cursor into a box.
@OptIn(ExperimentalTestApi::class)
class HookQuestionTest {
    @Test
    fun anOptionAnsweredInWordsAsksForThemAndSendsThemWithItsKey() = runComposeUiTest {
        showing(PLAN)
        assertEquals(0, fieldCount(FIELD))

        onNodeWithContentDescription("Answer 3, $CHANGE").performClick()
        waitForIdle()
        assertTrue(RecordingIo.sent.isEmpty(), "pressing the option answered it before anything was typed: ${RecordingIo.sent}")
        assertEquals(1, fieldCount(FIELD), "pressing the option offered nowhere to type")
        onNodeWithContentDescription(SEND).assertIsNotEnabled()

        onNodeWithContentDescription(FIELD, useUnmergedTree = true).performTextInput("split the migration")
        waitForIdle()
        onNodeWithContentDescription(SEND).assertIsEnabled().performClick()
        waitForIdle()
        assertEquals(listOf<ClientMsg>(ClientMsg.Answer(PANE_ID, "3", "q9", "split the migration")), RecordingIo.sent.toList())
    }

    @Test
    fun theFieldAndItsSendAreOnScreenWhenTheyOpenUnderFiveDescribedOptions() {
        for ((name, size) in listOf("desk" to DESK, "phone" to PORTRAIT)) {
            runComposeUiTest {
                showing(INDENT, size)
                onNodeWithContentDescription("Answer 5, Type something").performClick()
                mainClock.advanceTimeBy(1_000)
                waitForIdle()
                onNodeWithContentDescription("Your answer: Type something", useUnmergedTree = true).assertIsDisplayed()
                val send = onNodeWithContentDescription(SEND).assertIsDisplayed().getBoundsInRoot()
                val root = onRoot().getBoundsInRoot()
                assertTrue(send.bottom <= root.bottom && send.top >= root.top, "$name: Send is at $send, off a root of $root")
                val card = onNodeWithContentDescription(ASKING, substring = true).fetchSemanticsNode().boundsInRoot
                assertTrue(
                    onNodeWithContentDescription(SEND).fetchSemanticsNode().let { node -> node.boundsInRoot.bottom <= card.bottom + 0.5f && node.boundsInRoot.height > 1f },
                    "$name: Send is clipped by the card, which ends at ${card.bottom}",
                )
            }
        }
    }

    @Test
    fun escapePutsTheFieldAwayAndSendsNothing() = runComposeUiTest {
        showing(PLAN)
        onNodeWithContentDescription("Answer 3, $CHANGE").performClick()
        waitForIdle()
        onNodeWithContentDescription(FIELD, useUnmergedTree = true).performTextInput("never mind")
        onNodeWithContentDescription(FIELD, useUnmergedTree = true).performKeyInput { pressKey(Key.Escape) }
        waitForIdle()
        assertEquals(0, fieldCount(FIELD), "Escape left the field standing")
        assertTrue(RecordingIo.sent.isEmpty())
    }

    @Test
    fun onAQuestionTakingSeveralAnswersTheWordsAreWhatSubmitsTheTicks() = runComposeUiTest {
        showing(SUITES)
        onNodeWithContentDescription("Answer 3, Type something").performClick()
        waitForIdle()
        assertTrue(RecordingIo.sent.isEmpty(), "the words option ticked rather than asked for words")
        onNodeWithContentDescription("Your answer: Type something", useUnmergedTree = true).performTextInput("and lint")
        onNodeWithContentDescription(SEND).performClick()
        waitForIdle()
        assertEquals(listOf<ClientMsg>(ClientMsg.Answer(PANE_ID, "3", "q8", "and lint")), RecordingIo.sent.toList())
    }

    @Test
    fun aHookQuestionWithNoOptionsIsShownAndAnsweredAtTheTerminal() = runComposeUiTest {
        showing(FORM)
        assertTrue(onAllNodesWithText("Which environment", substring = true).fetchSemanticsNodes().isNotEmpty())
        assertTrue(onAllNodesWithText("staging-eu", substring = true).fetchSemanticsNodes().isNotEmpty())
        assertTrue(onAllNodesWithText("Answer this at the terminal", substring = true).fetchSemanticsNodes().isNotEmpty())
        assertEquals(0, onAllNodes(anAnswerChip).fetchSemanticsNodes().size)
        assertEquals(0, onAllNodesWithContentDescription(SEND).fetchSemanticsNodes().size)
    }

    @Test
    fun aLongPlanIsReadAsMarkdownToItsLastLine() = runComposeUiTest {
        showing(PLAN)
        assertTrue(
            onAllNodesWithText("## Steps", substring = true, useUnmergedTree = true).fetchSemanticsNodes().isEmpty(),
            "the plan's markdown was drawn as its source",
        )
        onNodeWithText("step 60 of the migration", substring = true, useUnmergedTree = true).performScrollTo().assertIsDisplayed()
    }
}
