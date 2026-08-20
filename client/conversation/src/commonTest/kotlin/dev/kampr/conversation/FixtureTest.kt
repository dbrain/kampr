package dev.kampr.conversation

import dev.kampr.conversation.md.MdBlock
import dev.kampr.conversation.md.parseMarkdown
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

private fun storeWith(vararg frames: String): KamprStore {
    val store = KamprStore()
    for (frame in frames) store.accept(assertNotNull(Wire.decode(frame), "undecodable frame"))
    return store
}

class FixtureTest {
    @Test
    fun claudeTranscriptCarriesAMarkdownTableThroughToTheRenderer() {
        val store = storeWith(CLAUDE_CONVO)
        val turns = store.pane("01JNODE.../w3:p2").turns
        assertEquals(5, turns.size)
        val prose = turns.flatMap { it.blocks }.filterIsInstance<Block.Md>().single { "|" in it.text }
        val table = parseMarkdown(prose.text).filterIsInstance<MdBlock.Table>().single()
        assertEquals(listOf("Key", "Accepted"), table.header)
        assertEquals(listOf(listOf("`Up`", "yes"), listOf("`PageUp`", "no")), table.rows)
    }

    @Test
    fun claudeEditTurnGroupsItsDiffUnderTheToolCall() {
        val store = storeWith(CLAUDE_CONVO)
        val turn = store.pane("01JNODE.../w3:p2").turns.first { it.blocks.any { b -> b is Block.Diff } }
        val call = groupBlocks(turn.blocks).single() as Piece.Call
        assertEquals("Edit", call.tool.name)
        assertEquals(1, call.detail.size)
        assertTrue(parseDiff((call.detail.single() as Block.Diff).text).any { it.kind == DiffKind.Added })
    }

    @Test
    fun codexApplyPatchEnvelopeIsClassifiedAsADiff() {
        val store = storeWith(CODEX_CONVO)
        val diff = store.pane("01JNODE.../w4:p1").turns
            .flatMap { it.blocks }.filterIsInstance<Block.Diff>().single()
        val lines = parseDiff(diff.text)
        assertEquals(DiffKind.Meta, lines.first().kind)
        assertEquals(1, lines.count { it.kind == DiffKind.Added })
        assertEquals(1, lines.count { it.kind == DiffKind.Removed })
    }

    // Probe #43: an unmatched custom_tool_call is Codex's pending signal, and it reaches the
    // client as a tool block that is still running.
    @Test
    fun codexUnansweredToolCallArrivesRunning() {
        val store = storeWith(CODEX_PENDING_CONVO)
        val tool = store.pane("01JNODE.../w4:p1").turns
            .flatMap { it.blocks }.filterIsInstance<Block.Tool>().single()
        assertEquals(TOOL_RUNNING, tool.state)
    }

    @Test
    fun aRunningToolIsRevisedInPlaceWhenItsResultLands() {
        val store = storeWith(RICH_CONVO)
        val pane = store.pane("01JNODE.../w3:p2")
        val before = pane.turns.single { it.blocks.filterIsInstance<Block.Tool>().any { t -> t.name == "Edit" } }
        assertEquals(TOOL_RUNNING, (before.blocks[0] as Block.Tool).state)
        val ids = pane.turns.map { it.id }

        store.accept(assertNotNull(Wire.decode(RICH_REVISION)))

        val after = pane.turns.single { it.id == before.id }
        assertEquals("done", (after.blocks[0] as Block.Tool).state)
        assertEquals(1, pane.turns.count { it.id == before.id })
        assertEquals(ids + listOf("a-0008"), pane.turns.map { it.id })
        val call = groupBlocks(after.blocks).single() as Piece.Call
        assertEquals(1, call.detail.size)
    }

    @Test
    fun pagingBackwardsWithTheOpaqueCursorPrependsTheOlderPage() {
        val store = storeWith(RICH_PAGE_TAIL)
        val pane = store.pane("01JNODE.../w3:p2")
        assertTrue(pane.convoMore)
        assertEquals("a-0003", pane.convoCursor)

        store.accept(assertNotNull(Wire.decode(RICH_PAGE_OLDER)))

        assertEquals(listOf("u-0001", "a-0002", "a-0003", "a-0005", "a-0006"), pane.turns.map { it.id })
        assertFalse(pane.convoMore)
    }

    @Test
    fun searchSpansTheWholeTranscriptNotJustTheVisibleTurns() {
        val store = storeWith(RICH_CONVO)
        val turns = store.pane("01JNODE.../w3:p2").turns
        assertEquals(listOf(1), searchHits(turns, "truncated"))
        assertEquals(listOf(3), searchHits(turns, "surface_geometry"))
        assertEquals(listOf(2, 4), searchHits(turns, "surface.rs"))
        assertTrue(searchHits(turns, "letterbox").isEmpty())
        assertTrue(searchHits(turns, "a").isEmpty())

        store.accept(assertNotNull(Wire.decode(RICH_REVISION)))
        assertEquals(listOf(4, 5), searchHits(turns, "letterbox"))
    }

    @Test
    fun theProbeLogTableSurvivesTheRoundTripAsFourColumns() {
        val store = storeWith(RICH_CONVO)
        val prose = store.pane("01JNODE.../w3:p2").turns[1].blocks.filterIsInstance<Block.Md>().first()
        val table = parseMarkdown(prose.text).filterIsInstance<MdBlock.Table>().single()
        assertEquals(listOf("#", "Claim", "How", "Result"), table.header)
        assertEquals(5, table.rows.size)
        assertTrue(table.rows.any { it[0] == "51" })
    }
}
