package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

private fun decode(frame: String): ServerMsg = Wire.decode(frame) ?: error("undecodable: $frame")

private const val LAUNCHING_TURN =
    """{"t":"convo","pane":"$PANE","cursor":"t1","more":false,"turns":[
        {"id":"t1","role":"assistant","blocks":[
          {"b":"tool","name":"Agent","summary":"Explore","state":"done"},
          {"b":"sub","id":"aGFuZGxl","kind":"Explore","title":"Map the manage op end-to-end path","depth":1}]}]}"""

private const val SUB_PAGE =
    """{"t":"convo","pane":"$PANE","sub":"aGFuZGxl","fresh":true,"more":false,"turns":[
        {"id":"s1","role":"user","blocks":[{"b":"md","text":"map the manage op path"}]},
        {"id":"s2","role":"assistant","blocks":[{"b":"md","text":"Six hops, and they are…"}]}]}"""

class SubConversationTest {
    @Test
    fun aTurnThatLaunchedAnAgentCarriesTheHandleToItsConversation() {
        val convo = decode(LAUNCHING_TURN) as ServerMsg.Convo
        val sub = convo.turns.single().blocks.filterIsInstance<Block.Sub>().single()
        assertEquals("aGFuZGxl", sub.id)
        assertEquals("Explore", sub.kind)
        assertEquals("Map the manage op end-to-end path", sub.title)
        assertEquals(1, sub.depth)
    }

    // Nothing to hand back is nothing to open, and a card that offers to open one is the inert
    // affordance this codebase has paid for before.
    @Test
    fun aSubBlockWithNoHandleIsNotABlock() {
        val convo = decode("""{"t":"convo","pane":"$PANE","turns":[
            {"id":"t1","role":"assistant","blocks":[{"b":"sub","kind":"Explore"}]}]}""") as ServerMsg.Convo
        assertTrue(convo.turns.single().blocks.none { it is Block.Sub })
    }

    // The one thing that must not happen: a launched conversation rendered as the pane's own
    // reply. Every page of one is `fresh`, so routing it into the pane's turns would not merely
    // add somebody else's words — it would clear the transcript and replace it with them.
    @Test
    fun aPageOfALaunchedConversationIsNeverThePanesOwnReply() {
        val store = KamprStore()
        store.accept(decode(LAUNCHING_TURN))
        store.accept(decode(SUB_PAGE))
        val pane = store.pane(PANE)
        assertEquals(listOf("t1"), pane.turns.map { it.id })
        val sub = assertNotNull(pane.subOrNull("aGFuZGxl"), "the page landed nowhere at all")
        assertEquals(listOf("s1", "s2"), sub.turns.map { it.id })
        assertTrue(sub.loaded)
    }

    @Test
    fun aPaneWithNoSubPageHasNoSubConversationYet() {
        val store = KamprStore()
        store.accept(decode(LAUNCHING_TURN))
        assertNull(store.pane(PANE).subOrNull("aGFuZGxl"))
    }

    // The handle is opaque and is only ever handed back.
    @Test
    fun openingOneAsksForItByTheHandleItWasGiven() {
        assertEquals(
            """{"t":"convo.sub","pane":"$PANE","id":"aGFuZGxl"}""",
            Wire.encode(ClientMsg.ConvoSub(PANE, "aGFuZGxl")),
        )
    }

    @Test
    fun aSecondPageOfOneAsksBeforeTheTurnItHolds() {
        assertEquals(
            """{"t":"convo.sub","pane":"$PANE","id":"aGFuZGxl","before":"s1"}""",
            Wire.encode(ClientMsg.ConvoSub(PANE, "aGFuZGxl", "s1")),
        )
    }

    // **A subagent's transcript grows while it runs**, and the reason to open one is to watch it
    // work — so what the node follows has to arrive in the conversation the reader opened, and
    // never in the pane's own. Appending it to the pane is the same defect the `sub` page shape
    // exists to prevent, and a growing transcript is the path it would happen on.
    @Test
    fun aLaunchedConversationGrowsWhereItWasOpenedAndNotInThePanesOwnTranscript() {
        val store = KamprStore()
        store.accept(decode(LAUNCHING_TURN))
        store.accept(decode(SUB_PAGE))
        val pane = store.pane(PANE)
        val ownTurns = pane.turns.size

        store.accept(
            decode(
                """{"t":"convo.turn","pane":"$PANE","sub":"aGFuZGxl","turns":[
                    {"id":"s3","role":"assistant","blocks":[{"b":"md","text":"and then took another step"}]}]}"""
            )
        )

        assertEquals(
            listOf("s1", "s2", "s3"),
            pane.sub("aGFuZGxl").turns.map { it.id },
            "the step the launched agent took did not arrive where it was being watched",
        )
        assertEquals(
            ownTurns,
            pane.turns.size,
            "a launched agent's step was filed as the pane's own, in the parent's voice",
        )
    }
}
