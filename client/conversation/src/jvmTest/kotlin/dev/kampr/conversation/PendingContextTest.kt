package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.Answering
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The two dialogs, exactly as the node publishes them off the real screens captured in
// `crates/kampr-node/tests/fixtures/dialogs/` (#421).
private val ASKED = ServerMsg.Pending(
    pane = PANE_ID,
    question = "Which indentation do you prefer?",
    header = "Indentation",
    multi = false,
    source = "screen",
    options = listOf(
        PendingOption("1", "Tabs", "Indent with tab characters."),
        PendingOption("2", "Two spaces", "Indent with two spaces per level."),
        PendingOption("3", "Four spaces", "Indent with four spaces per level."),
        PendingOption("4", "Type something."),
        PendingOption("5", "Chat about this"),
    ),
)

private val TICKED = ServerMsg.Pending(
    pane = PANE_ID,
    question = "Which test suites should I run?",
    header = "Test suites",
    multi = true,
    source = "screen",
    options = listOf(
        PendingOption("1", "unit", "Run the unit test suite.", chosen = true),
        PendingOption("2", "integration", "Run the integration test suite."),
        PendingOption("3", "browser", "Run the browser test suite.", chosen = true),
    ),
)

// The operator, on 0.1.50: *"at the moment we get options to select from with no context around
// them and the context is the most important part"*.
@OptIn(ExperimentalTestApi::class)
class PendingContextTest {
    private fun strip(
        pending: ServerMsg.Pending,
        answering: Answering = Answering.Ready,
        onAnswer: (String) -> Unit = {},
        onSubmit: (() -> Unit)? = {},
    ): @androidx.compose.runtime.Composable () -> Unit = {
        CompositionLocalProvider(LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone)) {
            Box(Modifier.size(411.dp, 914.dp)) {
                PendingStrip(pending, answering, onAnswer, Modifier.fillMaxWidth(), onSubmit)
            }
        }
    }

    @Test
    fun everyAnswerIsDrawnWithWhatTheDialogSaysItMeans() = runComposeUiTest {
        setContent(strip(ASKED))
        waitForIdle()

        onNodeWithText("Indentation").assertExists()
        onNodeWithText("Which indentation do you prefer?").assertExists()
        for (said in listOf(
            "Indent with tab characters.",
            "Indent with two spaces per level.",
            "Indent with four spaces per level.",
        )) {
            onNodeWithText(said).assertExists()
        }
    }

    // A reader who cannot see the screen gets the same thing: the title, the question, and every
    // option *with* its description rather than four bare names.
    @Test
    fun theSpokenCardCarriesTheDescriptionsToo() = runComposeUiTest {
        setContent(strip(ASKED))
        waitForIdle()
        val said = onNodeWithContentDescription("The agent is asking:", substring = true)
            .fetchSemanticsNode()
            .config[SemanticsProperties.ContentDescription]
            .first()

        assertTrue(said.startsWith("Indentation."), said)
        assertTrue(said.contains("1 Tabs. Indent with tab characters."), said)
        assertTrue(said.contains("3 Four spaces. Indent with four spaces per level."), said)
    }

    @Test
    fun pressingAnAnswerStillSendsItsKey() = runComposeUiTest {
        val sent = mutableListOf<String>()
        setContent(strip(ASKED, onAnswer = { sent += it }))
        waitForIdle()
        onNodeWithContentDescription("Answer 2, Two spaces").performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        assertEquals(listOf("2"), sent)
    }

    // The half that would otherwise be a lie: on a question that takes several answers a press is
    // a *tick*, and the card has to say so — in what it draws, in what it announces, and in the
    // line under it.
    @Test
    fun aQuestionThatTakesSeveralAnswersSaysAPressIsATick() = runComposeUiTest {
        setContent(strip(TICKED))
        waitForIdle()

        onNodeWithContentDescription("Untick unit").assertExists()
        onNodeWithContentDescription("Tick integration").assertExists()
        assertEquals(
            0,
            onAllNodesWithContentDescription("Answer 1,", substring = true).fetchSemanticsNodes().size,
            "a tick was offered as an answer",
        )
        onNodeWithText("Takes several answers — tick what you want, then send.").assertExists()
    }

    @Test
    fun sendingAMultipleAnswerQuestionIsItsOwnPressAndCountsWhatIsTicked() = runComposeUiTest {
        var submitted = 0
        setContent(strip(TICKED, onSubmit = { submitted += 1 }))
        waitForIdle()
        onNodeWithContentDescription("Send 2 answers").performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        assertEquals(1, submitted)
    }

    // Nothing ticked is nothing to send, and offering it would put a key sequence into a dialog
    // that would answer with the empty set.
    @Test
    fun sendIsRefusedUntilSomethingIsTicked() = runComposeUiTest {
        val none = TICKED.copy(options = TICKED.options.map { it.copy(chosen = false) })
        setContent(strip(none))
        waitForIdle()
        onNodeWithText("Tick what you want").assertExists()
        onNodeWithContentDescription("Send 0 answers").assertIsNotEnabled()
    }

    // A node that has never heard of any of this sends what it always sent, and the card is the
    // card it always was.
    @Test
    fun anOlderNodesPendingFrameStillDrawsAsASingleAnswerQuestion() {
        val frame = Wire.decode(
            """{"t":"pending","pane":"$PANE_ID","question":"Do you want to make this edit?",
               "options":[{"key":"1","label":"Yes"},{"key":"2","label":"No"}],"source":"screen"}"""
        ) as ServerMsg.Pending

        assertEquals(null, frame.header)
        assertTrue(!frame.multi)
        assertTrue(frame.options.all { it.detail == null && !it.chosen })
    }

    // And a node that does send it round-trips, so the field names on the two halves agree.
    @Test
    fun theNewFieldsSurviveTheWire() {
        val frame = Wire.decode(
            """{"t":"pending","pane":"$PANE_ID","question":"Which test suites should I run?",
               "header":"Test suites","multi":true,"source":"screen","options":[
                 {"key":"1","label":"unit","detail":"Run the unit test suite.","chosen":true},
                 {"key":"2","label":"integration","detail":"Run the integration test suite."}]}"""
        ) as ServerMsg.Pending

        assertEquals("Test suites", frame.header)
        assertTrue(frame.multi)
        assertEquals("Run the unit test suite.", frame.options[0].detail)
        assertTrue(frame.options[0].chosen)
        assertTrue(!frame.options[1].chosen)
    }
}
