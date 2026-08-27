package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test

private const val OPENS = "The width inference lands on 93 columns."
private const val ENDS = "And the last line of it is this one."
private const val PIN = "Put away the message of"

// Taller than the phone it is read on, which is the whole condition for pinning anything: the
// header of a message this long is off the top of the screen by the time its end is on it.
private val LONG = Turn(
    "a-2", "assistant", "2026-08-23T09:00:00.000Z",
    listOf(
        Block.Md(
            (listOf(OPENS) + (1..60).map { "Line $it of the answer, long enough to hold the column." } + ENDS)
                .joinToString("\n\n"),
        ),
    ),
)

private val SHORT = Turn("a-3", "assistant", "2026-08-23T09:00:00.000Z", listOf(Block.Md("Done.")))

private fun storeOf(vararg turns: Turn): KamprStore {
    val store = KamprStore()
    store.accept(ServerMsg.Convo(pane = PANE_ID, cursor = turns.first().id, more = false, turns = turns.toList()))
    return store
}

@Composable
private fun Transcript(store: KamprStore) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
    ) {
        Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
            ConversationView(store.pane(PANE_ID), demoInfo(), Modifier.fillMaxSize())
        }
    }
}

@OptIn(ExperimentalTestApi::class)
class PinnedTurnTest {
    // The reader is standing in the middle of a message whose own header is above the fold, and
    // the control that puts it away went with it. Pinning it is what makes a long answer
    // dismissable from where the reader actually is.
    @Test
    fun theHeaderOfTheMessageTheReaderIsStandingInStaysOnScreen() = runComposeUiTest {
        setContent { Transcript(storeOf(LONG)) }
        waitForIdle()
        onNodeWithText(ENDS, substring = true).assertIsDisplayed()
        // The message's own header went off the top with the rest of it — the copy of its first
        // line still on screen is the pinned bar's, and it is on screen because the bar is.
        onNodeWithContentDescription(PIN, substring = true).assertIsDisplayed()
        onAllNodesWithText(OPENS, substring = true).assertCountEquals(2)
    }

    @Test
    fun thePinnedHeaderPutsTheMessageAwayFromWhereTheReaderIs() = runComposeUiTest {
        setContent { Transcript(storeOf(LONG)) }
        waitForIdle()
        onNodeWithContentDescription(PIN, substring = true).performClick()
        waitForIdle()
        onAllNodesWithText(ENDS, substring = true).assertCountEquals(0)
        // Folded, so its own header is back on screen — and a folded row is not one the reader can
        // be standing in the middle of, so nothing is pinned over it.
        onAllNodesWithText(OPENS, substring = true).assertCountEquals(1)
        onAllNodesWithContentDescription(PIN, substring = true).assertCountEquals(0)
    }

    // A bar that appears when there is nothing above the fold is chrome that says nothing, and it
    // costs the transcript a row of its own height to say it.
    @Test
    fun aTranscriptThatFitsPinsNothing() = runComposeUiTest {
        setContent { Transcript(storeOf(SHORT)) }
        waitForIdle()
        onAllNodesWithContentDescription(PIN, substring = true).assertCountEquals(0)
    }
}
