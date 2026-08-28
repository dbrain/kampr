package dev.kampr.conversation

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.semantics.SemanticsProperties
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val AT_THE_DESK = "push the branch when"
private const val CTRL_C = "\u0003"
private const val TAKE_OVER = "Take that line off the pane and put it in this reply box"
private const val REPLY = "Reply to claude"

private val TYPED =
    """{"t":"convo.composer","pane":"$PANE_ID","text":"$AT_THE_DESK","clear":"$CTRL_C"}"""
private val UNMEASURED =
    """{"t":"convo.composer","pane":"$PANE_ID","text":"$AT_THE_DESK"}"""
private const val EMPTIED = """{"t":"convo.composer","pane":"$PANE_ID","text":null}"""

@OptIn(ExperimentalTestApi::class)
class DeskLineScreenTest {
    // The report, end to end: `input` is herdr's `pane.send_text` and it appends to whatever is
    // already on the pane's line, so a sentence begun at the desk and a reply sent from here
    // submit as one run-on line — and nothing on this screen had ever shown the first half of it,
    // which is what made it surprising rather than merely occasionally wrong.
    @Test
    fun theLineWaitingAtTheDeskIsOnTheScreenAboveTheBoxThatWouldBeAddedToIt() = runComposeUiTest {
        val (store, pane) = demoPane(RICH_CONVO)
        store.accept(requireNotNull(Wire.decode(TYPED)))
        setContent { Harnessed(pane) }
        onNodeWithText(AT_THE_DESK).assertIsDisplayed()
        onAllNodesWithText("added to the end", substring = true).assertCountEquals(1)
    }

    // Pressing it *moves* the line: the words land in this box before the pane is asked to let go
    // of them, and the clearing keystroke that goes out is the one the node measured for that
    // harness — never one chosen here, on a machine this client has never seen.
    @Test
    fun takingTheLineOverPutsItInTheReplyBoxAndSendsTheMeasuredClearingKey() = runComposeUiTest {
        val (store, pane) = demoPane(RICH_CONVO)
        store.accept(requireNotNull(Wire.decode(TYPED)))
        RecordingIo.sent.clear()
        setContent { Harnessed(pane) }
        onNodeWithContentDescription(TAKE_OVER).performClick()
        waitForIdle()
        assertEquals(
            AT_THE_DESK,
            onNodeWithContentDescription(REPLY)
                .fetchSemanticsNode()
                .config[SemanticsProperties.EditableText]
                .text,
            "the line was cleared off the pane without landing in the box, which loses it",
        )
        assertEquals(
            listOf(ClientMsg.InputText(PANE_ID, CTRL_C)),
            RecordingIo.sent.toList(),
            "the pane was not asked to let go of the line, or was asked with the wrong key",
        )
    }

    // **Looking is not taking.** Composing the view sends nothing at all: a write that empties
    // somebody's half-written sentence is the last thing to do as a consequence of opening a
    // screen, and the one deliberate path is a press.
    @Test
    fun openingTheConversationNeverTouchesTheLineByItself() = runComposeUiTest {
        val (store, pane) = demoPane(RICH_CONVO)
        store.accept(requireNotNull(Wire.decode(TYPED)))
        RecordingIo.sent.clear()
        setContent { Harnessed(pane) }
        waitForIdle()
        assertTrue(
            RecordingIo.sent.isEmpty(),
            "merely looking at the pane wrote to it: ${RecordingIo.sent}",
        )
    }

    // A harness nobody has measured a clearing keystroke for still shows its line — that half is
    // free and is most of the value — and offers no button, rather than a disabled one or one
    // wired to a guess.
    @Test
    fun aHarnessWithNoMeasuredClearingKeyShowsTheLineAndOffersNoButton() = runComposeUiTest {
        val (store, pane) = demoPane(RICH_CONVO)
        store.accept(requireNotNull(Wire.decode(UNMEASURED)))
        setContent { Harnessed(pane) }
        onNodeWithText(AT_THE_DESK).assertIsDisplayed()
        onAllNodesWithContentDescription(TAKE_OVER).assertCountEquals(0)
    }

    // `text: null` is how the strip comes down. Without it this screen would go on claiming a line
    // the operator emptied at the desk minutes earlier.
    @Test
    fun aBoxTheDeskHasEmptiedTakesTheStripDown() = runComposeUiTest {
        val (store, pane) = demoPane(RICH_CONVO)
        store.accept(requireNotNull(Wire.decode(TYPED)))
        store.accept(requireNotNull(Wire.decode(EMPTIED)))
        setContent { Harnessed(pane) }
        onAllNodesWithText(AT_THE_DESK).assertCountEquals(0)
    }
}

@OptIn(ExperimentalTestApi::class)
@androidx.compose.runtime.Composable
private fun Harnessed(pane: dev.kampr.shared.model.PaneState) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
    ) {
        ConversationView(pane, demoInfo(status = "idle"), Modifier.fillMaxSize())
    }
}
