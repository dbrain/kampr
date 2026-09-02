package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
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
import kotlin.test.assertTrue

private const val TWO_LINES = "the fourth hop drops it\nthe other three are fine"

private fun paneSaying(vararg turns: Turn): PaneState {
    val store = KamprStore()
    store.accept(ServerMsg.Convo(pane = PANE_ID, cursor = "z", more = false, turns = turns.toList()))
    return store.pane(PANE_ID)
}

@Composable
private fun Screen(pane: PaneState) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
    ) {
        Box(Modifier.fillMaxSize()) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) }
    }
}

// Reported from a phone: the operator's own messages lost every line break they were typed with.
// The diagnosis was right — the prose is markdown, and CommonMark says a single newline inside a
// paragraph is a space. That is the agent's rule, not the person's.
@OptIn(ExperimentalTestApi::class)
class TypedProseTest {
    @Test
    fun the_breaks_a_person_typed_survive_and_the_ones_an_agent_wrapped_do_not() = runComposeUiTest {
        setContent {
            Screen(
                paneSaying(
                    Turn("u-1", "user", "2026-08-24T09:00:00.000Z", listOf(Block.Md(TWO_LINES))),
                    Turn("a-1", "assistant", "2026-08-24T09:00:06.000Z", listOf(Block.Md(TWO_LINES))),
                ),
            )
        }
        onNodeWithText(TWO_LINES).assertExists()
        onNodeWithText(TWO_LINES.replace('\n', ' ')).assertExists()
    }

    // Not solved by rendering a person's words as plain text: people paste fences, bullets and
    // tables into a reply and every one of them has to go on being what it is.
    @Test
    fun a_person_can_still_paste_markdown_into_a_reply() = runComposeUiTest {
        setContent {
            Screen(
                paneSaying(
                    Turn(
                        "u-1", "user", "2026-08-24T09:00:00.000Z",
                        listOf(Block.Md("run this:\n\n```sh\nkampr doctor\n```\n\n- and read #331\n")),
                    ),
                ),
            )
        }
        onNodeWithText("kampr doctor", substring = true).assertExists()
        onNodeWithText("sh").assertExists()
        onNodeWithText("and read #331", substring = true).assertExists()
        assertTrue(
            onAllNodesWithText("```", substring = true).fetchSemanticsNodes().isEmpty(),
            "a person's fence was printed as its own punctuation instead of rendered",
        )
    }
}
