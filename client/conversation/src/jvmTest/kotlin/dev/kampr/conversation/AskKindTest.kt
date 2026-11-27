package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.v2.runComposeUiTest
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PendingField
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.Reply
import dev.kampr.shared.wire.ServerMsg
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val FORM = ServerMsg.Pending(
    pane = PANE_ID,
    id = "form1",
    header = "probe",
    question = "Pick a colour",
    source = "transcript",
    kind = "form",
    options = listOf(PendingOption("1", "Submit"), PendingOption("3", "Decline")),
    fields = listOf(
        PendingField("colour", title = "Colour", required = true, choices = listOf("red", "teal")),
        PendingField("count", type = "integer", detail = "how many"),
        PendingField("loud", type = "boolean"),
    ),
)

private val FRUIT = ServerMsg.Pending(
    pane = PANE_ID,
    id = "ask1#0",
    question = "Which fruit?",
    source = "transcript",
    kind = "question",
    options = listOf(PendingOption("1", "Apple"), PendingOption("2", "Pear"), PendingOption("3", "Type something", text = true)),
)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.asked(question: ServerMsg.Pending) {
    RecordingIo.sent.clear()
    val store = KamprStore()
    store.accept(question)
    setContent {
        CompositionLocalProvider(
            LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
            LocalPaneIo provides RecordingIo,
            LocalConnectionStatus provides ConnectionStatus.Live("full"),
        ) {
            Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                ConversationView(store.pane(PANE_ID), demoInfo(), Modifier.fillMaxSize())
            }
        }
    }
    waitForIdle()
}

// What answering means is the node's word (`pending.kind`): a form is filled, and only a
// permission's key carries the allow or deny it means.
@OptIn(ExperimentalTestApi::class)
class AskKindTest {
    @Test
    fun aFormsSubmitOpensItsFieldsAndSendsTheirValues() = runComposeUiTest {
        asked(FORM)
        assertTrue(onAllNodesWithText("Colour (colour) · string · required · red|teal", substring = true).fetchSemanticsNodes().isNotEmpty())

        onNodeWithContentDescription("Answer 1, Submit").performClick()
        waitForIdle()
        assertTrue(RecordingIo.sent.isEmpty(), "pressing Submit sent the form unfilled: ${RecordingIo.sent}")
        onNodeWithContentDescription("Send the form").assertIsNotEnabled()

        onNodeWithContentDescription("Form field: count", useUnmergedTree = true).performTextInput("3")
        onNodeWithContentDescription("Send the form").assertIsNotEnabled()
        onNodeWithContentDescription("Choose red for Colour").performClick()
        onNodeWithContentDescription("Choose teal for Colour").performClick()
        // The card is capped by the pane, so the last field is scrolled to as a reader would.
        onNodeWithContentDescription("Choose yes for loud").performScrollTo().performClick()
        waitForIdle()
        onNodeWithContentDescription("Send the form").assertIsEnabled().performClick()
        waitForIdle()
        val values = JsonObject(
            mapOf("colour" to JsonPrimitive("teal"), "count" to JsonPrimitive(3), "loud" to JsonPrimitive(true)),
        )
        assertEquals(listOf<ClientMsg>(ClientMsg.Answer(PANE_ID, "1", "form1", reply = Reply.Form(values))), RecordingIo.sent.toList())
    }

    // A field with fixed choices is chips, never a box to type a wrong answer into.
    @Test
    fun aFieldWithFixedChoicesIsChipsAndASecondTapClearsIt() = runComposeUiTest {
        asked(FORM)
        onNodeWithContentDescription("Answer 1, Submit").performClick()
        waitForIdle()
        listOf("Form field: Colour", "Form field: loud").forEach {
            assertTrue(onAllNodesWithContentDescription(it, useUnmergedTree = true).fetchSemanticsNodes().isEmpty(), "$it is a text box")
        }
        onNodeWithContentDescription("Choose teal for Colour").performClick()
        waitForIdle()
        onNodeWithContentDescription("Send the form").assertIsEnabled()
        // Two clicks in the same instant are one gesture to combinedClickable; a person's are not.
        mainClock.advanceTimeBy(1000)
        onNodeWithContentDescription("Choose teal for Colour").performClick()
        waitForIdle()
        onNodeWithContentDescription("Send the form").assertIsNotEnabled()
    }

    @Test
    fun aFormsDeclineIsItsKey() = runComposeUiTest {
        asked(FORM)
        onNodeWithContentDescription("Answer 3, Decline").performClick()
        waitForIdle()
        assertEquals(listOf<ClientMsg>(ClientMsg.Answer(PANE_ID, "3", "form1")), RecordingIo.sent.toList())
    }

    @Test
    fun aQuestionsKeyCarriesNoPermissionReply() = runComposeUiTest {
        asked(FRUIT)
        onNodeWithContentDescription("Answer 2, Pear").performClick()
        waitForIdle()
        assertEquals(listOf<ClientMsg>(ClientMsg.Answer(PANE_ID, "2", "ask1#0")), RecordingIo.sent.toList())
    }
}
