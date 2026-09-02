package dev.kampr.conversation

import dev.kampr.shared.util.parseIsoMillis
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertNull

private const val BEGAN = "2026-08-20T09:00:00Z"
private val AT = requireNotNull(parseIsoMillis(BEGAN))

private fun tool(id: String, name: String, state: String, at: String? = BEGAN) =
    Turn(id, "assistant", at, listOf(Block.Tool(name, "something", null, state)))

class WorkingStripTest {
    // Named by what the transcript says is happening, never by a word invented to fill the line.
    @Test
    fun theVerbIsWhateverTheAgentIsActuallyDoing() {
        assertEquals("Bash", workingVerb(Reply(listOf(tool("a", "Bash", TOOL_RUNNING)))))
        assertEquals("writing", workingVerb(Reply(listOf(proseTurn(LIVE_TURN_ID, "half a sen")))))
        assertEquals("working", workingVerb(Reply(listOf(tool("a", "Bash", "done")))))
        assertEquals("working", workingVerb(Reply(listOf(proseTurn("a", "a finished sentence")))))
        assertEquals("working", workingVerb(null))
    }

    // A call is marked running until a record says otherwise, and no record is written when a
    // harness is killed — so "running" is a claim that goes stale, and a preview scraped off the
    // pane this second is not. Whichever came last is the one that is still true.
    @Test
    fun whicheverCameLastIsTheOneItNames() {
        val ran = tool("a-1", "Bash", TOOL_RUNNING)
        val wrote = proseTurn(LIVE_TURN_ID, "and now I will explain")
        assertEquals("writing", workingVerb(Reply(listOf(ran, wrote))))
        assertEquals("Bash", workingVerb(Reply(listOf(wrote.copy(id = "a-0"), ran))))
    }

    // Nothing on the wire says when an agent started. Its reply's first record was written when it
    // did, which answers the same question without a new field — and answers it for a device that
    // only just connected, where a stopwatch started on arrival would read zero.
    @Test
    fun theClockRunsFromTheReplysOwnFirstRecord() {
        val reply = Reply(listOf(tool("a-1", "Bash", TOOL_RUNNING), tool("a-2", "Bash", TOOL_RUNNING, null)))
        assertEquals("0s", workingSince(reply, AT))
        assertEquals("41s", workingSince(reply, AT + 41_000))
        assertEquals("2m 11s", workingSince(reply, AT + 131_000))
        // Past the hour it keeps its seconds place, because this counter is watched for movement
        // and an agent an hour into a reply is the one being watched hardest.
        assertEquals("1h 05m 00s", workingSince(reply, AT + 3_900_000))
        assertNotEquals(
            workingSince(reply, AT + 3_900_000),
            workingSince(reply, AT + 3_901_000),
            "an hour-old reply's counter stood still for a minute at a time",
        )
    }

    // A reply the harness stamped in the future is two clocks disagreeing, not a negative
    // duration, and a counter that reads "-4s" is worse than one that is not drawn.
    @Test
    fun aReplyWithNoUsableStampSaysNothingRatherThanSomethingWrong() {
        assertNull(workingSince(Reply(listOf(tool("a", "Bash", TOOL_RUNNING, null))), AT))
        assertNull(workingSince(null, AT))
        assertNull(workingSince(Reply(listOf(tool("a", "Bash", TOOL_RUNNING))), AT - 4000))
    }
}
