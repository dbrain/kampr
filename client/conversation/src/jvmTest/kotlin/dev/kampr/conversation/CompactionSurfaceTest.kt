package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PROMPT = "carry on where you left off"
private const val ANSWERED = "Picking the width inference back up."
private const val INSIDE = "ran out of context"

private fun compacted(): KamprStore {
    val store = KamprStore()
    store.accept(
        ServerMsg.Convo(
            pane = PANE_ID, cursor = "u-1", more = false,
            turns = listOf(
                proseTurn("u-1", PROMPT, role = "user"),
                summaryTurn("u-2"),
                proseTurn("a-3", ANSWERED),
            ),
        ),
    )
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
class CompactionSurfaceTest {
    // `/compact` writes the harness's summary of everything it dropped back into the transcript as
    // a **user** record (#259), and the view rendered it as one: several screens of prose the
    // operator was told they had written. It is theirs to read and it is not theirs, so it keeps
    // its place under its own name — and it starts shut, because a summary is not what the reader
    // opened the conversation to see.
    @Test
    fun aCompactionSummaryIsNotInTheOperatorsVoiceAndIsPutAwayBeforeTheyArrive() = runComposeUiTest {
        setContent { Transcript(compacted()) }
        waitForIdle()

        onNodeWithText("compacted", substring = true).assertExists()
        assertEquals(
            0,
            onAllNodesWithText(INSIDE, substring = true).fetchSemanticsNodes().size,
            "the summary was spelled out rather than put away",
        )
        onNodeWithText(PROMPT, substring = true).assertExists()
        onNodeWithText(ANSWERED, substring = true).assertExists()
    }

    // Shut is not gone: the header is the same control every other turn wears, and it opens.
    @Test
    fun theHeaderOfAShutSummaryOpensItLikeAnyOtherFold() = runComposeUiTest {
        setContent { Transcript(compacted()) }
        waitForIdle()

        onNodeWithContentDescription("Show the message of compacted", substring = true).performClick()
        waitForIdle()

        assertTrue(
            onAllNodesWithText(INSIDE, substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the summary could not be opened again",
        )
    }
}
