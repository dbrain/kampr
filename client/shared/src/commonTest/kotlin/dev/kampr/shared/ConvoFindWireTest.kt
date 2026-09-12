package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

// The transcript search, both ways over the wire. It is a verb that owes an answer, so the three
// things worth pinning are the ask, the answer, and the promise that an answer is coming.
class ConvoFindWireTest {
    @Test
    fun theAskCarriesThePaneAndTheQuery() {
        assertEquals(
            """{"t":"convo.find","pane":"$PANE","query":"the width inference"}""",
            Wire.encode(ClientMsg.ConvoFind(PANE, "the width inference")),
        )
    }

    @Test
    fun theAnswerCarriesEveryMatchingTurnAndTheOnesItListed() {
        val found = Wire.decode(
            """{"t":"convo.find","pane":"$PANE","query":"scrollbar","total":41,"matches":[
                {"turn":"a-98","role":"assistant","at":"2026-09-12T02:14:08Z","from_end":2,"hits":3,
                 "text":"the scrollbar column is the one it keeps back"}]}""",
        )
        val msg = found as ServerMsg.ConvoFound
        assertEquals(41, msg.total, "the count is every match, not the listed ones")
        assertEquals(1, msg.matches.size)
        val hit = msg.matches.first()
        assertEquals("a-98", hit.turn, "the turn is the coordinate a client aims at")
        assertEquals(2, hit.fromEnd)
        assertEquals(3, hit.hits)
        assertEquals("assistant", hit.role)
    }

    // Additive by rule: a node that says nothing about the fields this client reads is a node this
    // client still talks to, and the missing ones read as their defaults rather than as a refusal.
    @Test
    fun aMatchWithNothingButATurnStillDecodes() {
        val msg = Wire.decode("""{"t":"convo.find","pane":"$PANE","matches":[{"turn":"a-1"}]}""")
        val hit = (msg as ServerMsg.ConvoFound).matches.single()
        assertEquals("a-1", hit.turn)
        assertEquals(0, hit.fromEnd)
        assertNull(hit.at)
        assertEquals("", msg.query)
    }

    // The promise, read off the greeting and separately from the scrollback search beside it: a
    // node can answer one and never have heard of the other.
    @Test
    fun theGreetingPromisesTheTwoSearchesApart() {
        val hello = Wire.decode(
            """{"t":"hello","protocol":1,"node_id":"01JNODE","node_name":"n","build":"0.1.81",
                "role":"full","caps":{"find":true},"security":{}}""",
        ) as ServerMsg.Hello
        assertTrue(hello.caps.find, "the node said it answers find")
        assertTrue(!hello.caps.convoFind, "and said nothing about searching a transcript")

        val newer = Wire.decode(
            """{"t":"hello","protocol":1,"node_id":"01JNODE","node_name":"n","build":"0.1.81",
                "role":"full","caps":{"find":true,"convo.find":true},"security":{}}""",
        ) as ServerMsg.Hello
        assertTrue(newer.caps.convoFind)
    }

    // A result set names turns, so it belongs to the transcript it was searched in. A fresh page is
    // the node saying this is a different transcript — the same rule that clears the turns.
    @Test
    fun aFreshPageTakesTheSearchWithIt() {
        val store = KamprStore()
        store.accept(
            ServerMsg.Convo(pane = PANE, cursor = "a-1", more = false, turns = listOf(Turn("a-1", "assistant", null, listOf(Block.Md("said")))))
        )
        store.accept(ServerMsg.ConvoFound(pane = PANE, query = "said", matches = emptyList(), total = 1))
        assertEquals(1, store.pane(PANE).convoFound?.total)

        store.accept(ServerMsg.Convo(pane = PANE, cursor = "b-1", more = false, turns = emptyList(), fresh = true))
        assertNull(
            store.pane(PANE).convoFound,
            "a search naming turns of the transcript the pane has left cannot aim at anything in this one",
        )
    }
}
