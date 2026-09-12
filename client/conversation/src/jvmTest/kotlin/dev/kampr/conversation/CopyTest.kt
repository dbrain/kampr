package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ClipboardManager
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.AnnotatedString
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals

@Suppress("DEPRECATION")
private class Pasteboard : ClipboardManager {
    var held: AnnotatedString? = null
    override fun setText(annotatedString: AnnotatedString) {
        held = annotatedString
    }
    override fun getText(): AnnotatedString? = held
    override fun hasText(): Boolean = held != null
}

private const val QUOTED =
    "Herdr's `truncated` means there was more than you asked for.\nA short read can be truncated too."

private val QUOTING = Turn(
    "a-1",
    "assistant",
    "2026-08-24T09:00:06.000Z",
    listOf(Block.Md("It says so outright:\n\n> " + QUOTED.replace("\n", "\n> ") + "\n")),
)

private const val OPENING = "The off-by-one is in `window`: it uses min where max was meant."
private const val CLOSING = "Fixed, and the test that caught it:\n\n```kotlin\nassertEquals(3, window(4))\n```"
private const val SIGNING_OFF = "Both suites are green."
private const val COPY_REPLY = "Copy the last message of the reply of"
private const val COPY_PINNED = "Copy the last message of the reply you are inside"

private val ASKING = Turn("u-1", "user", "2026-08-24T09:00:00.000Z", listOf(Block.Md("why is the window short?")))

private fun calling(id: String) = Turn(
    id,
    "assistant",
    "2026-08-24T09:00:03.000Z",
    listOf(Block.Tool("Bash", "run the tests", 1, null), Block.Code(null, "1 passed", TOOL_OUTPUT)),
)

private fun saying(id: String, vararg text: String) =
    Turn(id, "assistant", "2026-08-24T09:00:04.000Z", text.map { Block.Md(it) })

// The last thing said is not the last step: an agent that answers and then runs one more check
// ends its reply on a call, and the answer is still the message above it.
private val ANSWERING = arrayOf(
    ASKING,
    saying("a-1", OPENING),
    calling("a-2"),
    saying("a-3", CLOSING, SIGNING_OFF),
    calling("a-4"),
)

private val LONG_ANSWERING = arrayOf(
    ASKING,
    saying("a-1", (1..60).joinToString("\n\n") { "Line $it of the working, long enough to hold the column." }),
    calling("a-2"),
    saying("a-3", CLOSING),
)

private fun paneOf(vararg turns: Turn): PaneState {
    val store = KamprStore()
    store.accept(ServerMsg.Convo(pane = PANE_ID, cursor = "q", more = false, turns = turns.toList()))
    return store.pane(PANE_ID)
}

@Composable
@Suppress("DEPRECATION")
private fun Screen(board: Pasteboard, pane: PaneState) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
        LocalClipboardManager provides board,
    ) {
        Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
            ConversationView(pane, demoInfo(), Modifier.fillMaxSize())
        }
    }
}

@OptIn(ExperimentalTestApi::class)
class CopyTest {
    // A quote is the one block a reader wants out of the transcript verbatim and cannot select
    // cleanly on a phone. What comes off the button is what was quoted: no `> ` down the left, and
    // the line breaks the writer wrote rather than the ones the paragraph was laid out with.
    @Test
    fun copying_a_quote_yields_the_words_without_the_markers_that_marked_them() = runComposeUiTest {
        val board = Pasteboard()
        setContent { Screen(board, paneOf(QUOTING)) }
        onNodeWithContentDescription("Copy the quote").performClick()
        assertEquals(QUOTED, board.held?.text)
        onNodeWithContentDescription("Copied").assertIsDisplayed()
    }

    // What a reader copying "the answer" wants is the message the reply ended on, as the markdown
    // it was written in — not the notes the agent left on its way there, and not its tool output.
    @Test
    fun copying_a_reply_yields_the_last_thing_it_said_and_nothing_before_it() = runComposeUiTest {
        val board = Pasteboard()
        setContent { Screen(board, paneOf(*ANSWERING)) }
        onNodeWithContentDescription(COPY_REPLY, substring = true).performClick()
        assertEquals("$CLOSING\n\n$SIGNING_OFF", board.held?.text)
        onNodeWithContentDescription("Copied").assertIsDisplayed()
    }

    @Test
    fun a_reply_that_said_nothing_offers_nothing_to_copy() = runComposeUiTest {
        setContent { Screen(Pasteboard(), paneOf(ASKING, calling("a-1"))) }
        onNodeWithContentDescription("Put away the reply of", substring = true).assertExists()
        onNodeWithContentDescription(COPY_REPLY, substring = true).assertDoesNotExist()
        onNodeWithContentDescription(COPY_PINNED, substring = true).assertDoesNotExist()
    }

    // The reader finishes a long answer at its end, where its own head is a screen or more above
    // them; the bar pinned in its place is where the copy has to be.
    @Test
    fun a_long_reply_is_copied_from_the_bar_pinned_where_the_reader_is() = runComposeUiTest {
        val board = Pasteboard()
        setContent { Screen(board, paneOf(*LONG_ANSWERING)) }
        waitForIdle()
        onNodeWithText("assertEquals(3, window(4))", substring = true).assertIsDisplayed()
        onNodeWithContentDescription(COPY_PINNED, substring = true).assertIsDisplayed().performClick()
        assertEquals(CLOSING, board.held?.text)
    }
}
