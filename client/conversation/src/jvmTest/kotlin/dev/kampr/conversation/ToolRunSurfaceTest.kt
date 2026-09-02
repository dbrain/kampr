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
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test

private const val FAILING_RUN = "3 tool calls, Bash, 1 failed"
private const val RUNNING_RUN = "3 tool calls, Bash, Read, 1 running"
private const val FIRST_CARD = "Bash, cargo fmt --all -- --check"
private const val CARD_DETAIL = "Copy the bash block"

@Composable
private fun Transcript(store: KamprStore) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
    ) {
        Box(Modifier.size(PORTRAIT.first, 1400.dp)) {
            ConversationView(store.pane(PANE_ID), demoInfo(), Modifier.fillMaxSize())
        }
    }
}

private fun storeOfRuns(): KamprStore {
    val store = KamprStore()
    store.accept(ServerMsg.Convo(pane = PANE_ID, cursor = "r-1", more = false, turns = TOOL_RUN_TURNS))
    return store
}

@OptIn(ExperimentalTestApi::class)
class ToolRunSurfaceTest {
    // Two levels, and the request was explicit about it: "expand the group then expand each call".
    // A group that opened straight onto every tool's output would be the wall of cards again.
    @Test
    fun aCollapsedRunOpensToItsCardsAndEachCardStillOpensItsOwnOutput() = runComposeUiTest {
        setContent { Transcript(storeOfRuns()) }
        waitForIdle()
        onAllNodesWithContentDescription(FIRST_CARD, substring = true).assertCountEquals(0)

        onNodeWithContentDescription("Show $FAILING_RUN").performClick()
        waitForIdle()
        onNodeWithContentDescription("Hide $FAILING_RUN").assertExists()
        onNodeWithContentDescription("Show what was sent to $FIRST_CARD", substring = true).assertExists()
        onAllNodesWithContentDescription(CARD_DETAIL).assertCountEquals(0)

        onNodeWithContentDescription("Show what was sent to $FIRST_CARD", substring = true).performClick()
        waitForIdle()
        onAllNodesWithContentDescription(CARD_DETAIL).assertCountEquals(1)
    }

    // A tool call that failed, or one still running, going quiet behind a tidy row is the shape of
    // #233: the broken half of a thing wearing the healthy half's face. Both runs, because they
    // hide different halves of it.
    @Test
    fun aCollapsedRunSaysHowManyWhichToolsAndWhatWentWrong() = runComposeUiTest {
        setContent { Transcript(storeOfRuns()) }
        waitForIdle()
        onNodeWithContentDescription("Show $FAILING_RUN").assertExists()
        onNodeWithContentDescription("Show $RUNNING_RUN").assertExists()
        onAllNodesWithText("1 failed", useUnmergedTree = true).assertCountEquals(1)
        onAllNodesWithText("1 running", useUnmergedTree = true).assertCountEquals(1)
    }

    // The transcript ticks whenever the agent writes, and a group keyed on anything the tick
    // renumbers folds itself back up under the reader on every frame the node sends.
    @Test
    fun anOpenRunIsStillOpenAfterTheTranscriptTicks() = runComposeUiTest {
        val store = storeOfRuns()
        setContent { Transcript(store) }
        waitForIdle()
        onNodeWithContentDescription("Show $FAILING_RUN").performClick()
        onNodeWithContentDescription("Show what was sent to $FIRST_CARD", substring = true).performClick()
        waitForIdle()

        store.accept(ServerMsg.ConvoTurn(PANE_ID, listOf(proseTurn("r-9", "All green apart from the lint."))))
        waitForIdle()
        onNodeWithContentDescription("Hide $FAILING_RUN").assertExists()
        onAllNodesWithContentDescription(CARD_DETAIL).assertCountEquals(1)
    }
}
