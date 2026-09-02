package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val BASH = Block.Tool("Bash", "run the tests", 13, "done")
private val COMMAND = Block.Code("sh", "cargo test --workspace")
private val RESULT = Block.Code(null, "running 3 tests\nok\ntest result: ok.", TOOL_OUTPUT)

// Two calls in one record, each with its own answer. Appending the answer to the *turn* rather than
// to the run is what would file the first call's result under the second, which is why the output
// block is the last block of the run it belongs to.
private const val TWO_CALLS = """{"t":"convo","pane":"01JNODE.../w3:p2","cursor":"t1","more":false,"turns":[
    {"id":"t1","role":"assistant","blocks":[
      {"b":"tool","name":"Bash","summary":"ls","lines":2,"state":"done"},
      {"b":"code","lang":"sh","text":"ls"},
      {"b":"code","text":"one\ntwo","role":"output"},
      {"b":"tool","name":"Grep","summary":"needle","lines":1,"state":"done"},
      {"b":"code","text":"a needle","role":"output"}]}]}"""

@Composable
private fun Card(
    tool: Block.Tool,
    detail: List<Block>,
    output: Block.Code?,
    expanded: Boolean,
) {
    CompositionLocalProvider(LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone)) {
        Box(Modifier.fillMaxSize()) {
            ToolCard(
                tool = tool,
                detail = detail,
                query = "",
                expanded = expanded,
                onToggle = {},
                modifier = Modifier.fillMaxWidth(),
                output = output,
            )
        }
    }
}

// Reported: "getting blocks that say bash 13 lines but expanding them shows 1?" Both halves were
// individually correct — the count is the size of what the tool wrote back, and the body was the
// command it was given — and the card read as a promise about the body. #173: a label that has to
// be guessed is a failed label.
@OptIn(ExperimentalTestApi::class)
class ToolOutputTest {
    @Test
    fun the_count_says_which_thing_it_is_counting() = runComposeUiTest {
        setContent { Card(BASH, listOf(COMMAND), output = null, expanded = false) }
        onNodeWithText("13 lines of output").assertExists()
        assertTrue(
            onAllNodesWithText("13 lines", substring = false).fetchSemanticsNodes().isEmpty(),
            "the bare count that has to be guessed at is still on the card",
        )
    }

    // A card with no result carried cannot offer one, and must not say it is offering one.
    @Test
    fun a_card_with_no_result_offers_the_call_and_says_so() = runComposeUiTest {
        setContent { Card(BASH, listOf(COMMAND), output = null, expanded = true) }
        onNodeWithContentDescription("Hide what was sent to Bash, run the tests, 13 lines of output")
            .assertExists()
        onNodeWithText("cargo test --workspace", substring = true).assertExists()
    }

    @Test
    fun a_card_carrying_the_result_shows_it_and_names_it() = runComposeUiTest {
        setContent { Card(BASH, listOf(COMMAND), output = RESULT, expanded = true) }
        onNodeWithContentDescription("Hide the output of Bash, run the tests, 13 lines of output")
            .assertExists()
        onNodeWithText("test result: ok.", substring = true).assertExists()
        onNodeWithText("cargo test --workspace", substring = true).assertExists()
        onNodeWithText("sent").assertExists()
        onNodeWithText("output").assertExists()
    }

    // The node caps what it carries and leaves `lines` as the true total, so a body shorter than
    // the count is a body that was cut — said the way the file viewer already says it.
    @Test
    fun a_result_the_node_capped_says_how_much_of_it_this_is() = runComposeUiTest {
        setContent { Card(BASH, emptyList(), output = RESULT, expanded = true) }
        onNodeWithText("showing the first 3 of 13 lines").assertExists()
    }

    @Test
    fun a_result_that_arrived_whole_claims_nothing_about_being_cut() = runComposeUiTest {
        setContent {
            Card(Block.Tool("Bash", "run the tests", 3, "done"), emptyList(), RESULT, expanded = true)
        }
        assertTrue(
            onAllNodesWithText("showing the first", substring = true).fetchSemanticsNodes().isEmpty(),
            "a whole result was footed as a truncated one",
        )
    }

    @Test
    fun a_card_with_nothing_under_it_is_not_a_control() = runComposeUiTest {
        setContent { Card(BASH, emptyList(), output = null, expanded = false) }
        onNodeWithContentDescription("Tool Bash, run the tests, 13 lines of output").assertExists()
        assertTrue(
            onAllNodesWithContentDescription("Show", substring = true).fetchSemanticsNodes().isEmpty(),
            "a card with nothing to open offered to open it",
        )
    }

    @Test
    fun the_line_count_of_a_result_is_what_a_reader_would_count() {
        assertEquals(0, linesOf(""))
        assertEquals(1, linesOf("one"))
        assertEquals(1, linesOf("one\n"))
        assertEquals(3, linesOf("one\ntwo\nthree\n"))
    }

    // The wire's own frame, end to end: the node's `role` reaches the piece the card is drawn from.
    @Test
    fun each_call_in_one_record_keeps_its_own_answer() {
        val convo = Wire.decode(TWO_CALLS) as ServerMsg.Convo
        val calls = groupBlocks(convo.turns.single().blocks).filterIsInstance<Piece.Call>()
        assertEquals(listOf("Bash", "Grep"), calls.map { it.tool.name })
        assertEquals("one\ntwo", calls[0].output?.text)
        assertEquals(listOf("ls"), calls[0].detail.map { (it as Block.Code).text })
        assertEquals("a needle", calls[1].output?.text)
        assertTrue(calls[1].detail.isEmpty())
    }

    // The common case, and it stays the common case: the node writes an output block for Bash, Glob
    // and Grep and for anything that failed, and codex and agy panes carry none at all yet.
    @Test
    fun a_call_the_node_wrote_no_result_for_carries_none() {
        val blocks = listOf(BASH, Block.Code("sh", "cargo test --workspace"))
        val call = groupBlocks(blocks).single() as Piece.Call
        assertEquals(null, call.output)
        assertEquals(1, call.detail.size)
    }
}
