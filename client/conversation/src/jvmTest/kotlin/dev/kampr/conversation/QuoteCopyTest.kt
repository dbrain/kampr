package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ClipboardManager
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onNodeWithContentDescription
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

private fun paneQuoting(): PaneState {
    val store = KamprStore()
    store.accept(ServerMsg.Convo(pane = PANE_ID, cursor = "q", more = false, turns = listOf(QUOTING)))
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
        Box(Modifier.fillMaxSize()) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) }
    }
}

// A quote is the one block a reader wants out of the transcript verbatim and cannot select
// cleanly on a phone. What comes off the button is what was quoted: no `> ` down the left, and
// the line breaks the writer wrote rather than the ones the paragraph was laid out with.
@OptIn(ExperimentalTestApi::class)
class QuoteCopyTest {
    @Test
    fun copying_a_quote_yields_the_words_without_the_markers_that_marked_them() = runComposeUiTest {
        val board = Pasteboard()
        setContent { Screen(board, paneQuoting()) }
        onNodeWithContentDescription("Copy the quote").performClick()
        assertEquals(QUOTED, board.held?.text)
        onNodeWithContentDescription("Copied").assertIsDisplayed()
    }
}
