package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PendingSend
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.HooksScope
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

private fun decode(frame: String): ServerMsg = Wire.decode(frame) ?: error("undecodable: $frame")

private fun json(text: String) = Wire.json.parseToJsonElement(text)

// docs/12 W11.
class ClaudeHooksWireTest {
    @Test
    fun aHookQuestionCarriesWhatItWouldActOn() {
        val asked = decode(
            """{"t":"pending","pane":"$PANE","question":"Run the tests","header":"Bash","source":"hook",
               "context":"cargo test --workspace","options":[{"key":"1","label":"Yes"},{"key":"3","label":"No"}]}""",
        ) as ServerMsg.Pending
        assertEquals("hook", asked.source)
        assertEquals("cargo test --workspace", asked.context)
    }

    @Test
    fun anOlderNodesQuestionHasNoContext() {
        val asked = decode(
            """{"t":"pending","pane":"$PANE","question":"Edit?","source":"screen","options":[{"key":"1","label":"Yes"}]}""",
        ) as ServerMsg.Pending
        assertNull(asked.context)
    }

    @Test
    fun heardAndHooksDecode() {
        assertEquals(
            ServerMsg.ConvoHeard(PANE, "run the tests"),
            decode("""{"t":"convo.heard","pane":"$PANE","text":"run the tests"}"""),
        )
        assertEquals(
            ServerMsg.ClaudeHooks(PANE, "stale", scope = "project", ask = true, project = "/home/u/dev/kampr"),
            decode("""{"t":"claude.hooks","pane":"$PANE","state":"stale","scope":"project","ask":true,"project":"/home/u/dev/kampr"}"""),
        )
        assertEquals(
            ServerMsg.ClaudeHooks(PANE, "absent", scope = null, ask = false, project = null),
            decode("""{"t":"claude.hooks","pane":"$PANE","state":"absent","ask":false}"""),
        )
        assertNull(Wire.decode("""{"t":"claude.hooks","pane":"$PANE","ask":true}"""))
        assertNull(Wire.decode("""{"t":"convo.heard","text":"x"}"""))
    }

    @Test
    fun theTwoOpsEncodeToWhatTheNodeReads() {
        val cases = listOf(
            ManageOp.ClaudeHooksInstall(PANE, HooksScope.User) to
                """{"t":"manage","op":"claude.hooks.install","at":"$PANE","mode":"user","rid":"r1"}""",
            ManageOp.ClaudeHooksInstall(PANE, HooksScope.Project) to
                """{"t":"manage","op":"claude.hooks.install","at":"$PANE","mode":"project","rid":"r1"}""",
            ManageOp.ClaudeHooksDecline(PANE) to
                """{"t":"manage","op":"claude.hooks.decline","at":"$PANE","rid":"r1"}""",
        )
        for ((op, want) in cases) assertEquals(json(want), json(Wire.encode(ClientMsg.Manage(op, "r1"))))
    }

    @Test
    fun theStoreKeepsTheNewestHooksStateOnThePane() {
        val store = KamprStore()
        store.accept(decode("""{"t":"claude.hooks","pane":"$PANE","state":"absent","ask":true,"project":"/p"}"""))
        assertEquals("absent", store.pane(PANE).hooks.state?.state)
        store.accept(decode("""{"t":"claude.hooks","pane":"$PANE","state":"current","scope":"user","ask":true}"""))
        assertEquals("current", store.pane(PANE).hooks.state?.state)
    }

    @Test
    fun anAckForTheAskSettlesItAndARefusalIsKeptInWords() {
        val store = KamprStore()
        val pane = store.pane(PANE)
        store.accept(decode("""{"t":"claude.hooks","pane":"$PANE","state":"absent","ask":true}"""))
        val sent = mutableListOf<ClientMsg>()

        pane.hooks.ask(ManageOp.ClaudeHooksInstall(PANE, HooksScope.User), sent::add)
        val rid = (sent.single() as ClientMsg.Manage).rid
        assertNotNull(rid)
        assertTrue(pane.hooks.asking)

        store.accept(ServerMsg.Managed("claude.hooks.install", ok = false, id = null, rid = "someone else's"))
        assertTrue(pane.hooks.asking, "another ask's answer settled this one")

        store.accept(ServerMsg.Managed("claude.hooks.install", ok = false, id = null, rid = rid, message = "settings.json is not valid JSON"))
        assertFalse(pane.hooks.asking)
        assertEquals("settings.json is not valid JSON", pane.hooks.refusal)

        pane.hooks.ask(ManageOp.ClaudeHooksInstall(PANE, HooksScope.User), sent::add)
        assertNull(pane.hooks.refusal)
        store.accept(ServerMsg.Managed("claude.hooks.install", ok = true, id = null, rid = (sent.last() as ClientMsg.Manage).rid))
        assertFalse(pane.hooks.asking)
        assertNull(pane.hooks.refusal)
        assertNull(pane.hooks.offered(readOnly = false), "an install the node took was offered again")
    }

    @Test
    fun aDroppedSocketDoesNotLeaveTheAskInFlightForEver() {
        val store = KamprStore()
        val pane = store.pane(PANE)
        store.accept(decode("""{"t":"claude.hooks","pane":"$PANE","state":"absent","ask":true}"""))
        pane.hooks.ask(ManageOp.ClaudeHooksDecline(PANE)) {}
        pane.markStale()
        assertFalse(pane.hooks.asking)
    }

    @Test
    fun itIsOfferedOnlyWhereThereIsSomethingToInstallAndTheOperatorHasNotSaidNever() {
        fun offered(frame: String, readOnly: Boolean = false): Boolean {
            val store = KamprStore()
            store.accept(decode(frame))
            return store.pane(PANE).hooks.offered(readOnly) != null
        }
        assertTrue(offered("""{"t":"claude.hooks","pane":"$PANE","state":"absent","ask":true}"""))
        assertTrue(offered("""{"t":"claude.hooks","pane":"$PANE","state":"stale","scope":"user","ask":true}"""))
        assertFalse(offered("""{"t":"claude.hooks","pane":"$PANE","state":"current","scope":"user","ask":true}"""))
        assertFalse(offered("""{"t":"claude.hooks","pane":"$PANE","state":"absent","ask":false}"""))
        assertFalse(offered("""{"t":"claude.hooks","pane":"$PANE","state":"absent","ask":true}""", readOnly = true))
        assertFalse(offered("""{"t":"claude.hooks","pane":"$PANE","state":"someday","ask":true}"""))
        assertNull(KamprStore().pane(PANE).hooks.offered(readOnly = false))
    }

    @Test
    fun theHarnessHearingASendMarksItDeliveredUntilItsRecordArrives() {
        val store = KamprStore()
        val pane = store.pane(PANE)
        val send = pane.sends.sent("run the tests\n", emptyList(), emptyList(), delivered = true, at = 0.0)

        store.accept(decode("""{"t":"convo.heard","pane":"$PANE","text":"something typed at the desk"}"""))
        assertEquals(PendingSend.State.Sending, pane.sends.all.single().state)

        store.accept(decode("""{"t":"convo.heard","pane":"$PANE","text":"run the tests"}"""))
        assertEquals(PendingSend.State.Delivered, pane.sends.all.single().state)

        pane.sends.unseen(send.id)
        assertEquals(PendingSend.State.Delivered, pane.sends.all.single().state, "a heard reply was flagged unseen")

        store.accept(ServerMsg.ConvoTurn(PANE, listOf(Turn("u1", "user", blocks = listOf(Block.Md("run the tests"))))))
        assertTrue(pane.sends.all.isEmpty(), "the record did not retire a delivered reply")
    }

    @Test
    fun oneHearingDeliversOneOfTwoIdenticalSends() {
        val store = KamprStore()
        val pane = store.pane(PANE)
        pane.sends.sent("again", emptyList(), emptyList(), delivered = true, at = 0.0)
        pane.sends.sent("again", emptyList(), emptyList(), delivered = true, at = 1.0)
        store.accept(decode("""{"t":"convo.heard","pane":"$PANE","text":"again"}"""))
        assertEquals(listOf(PendingSend.State.Delivered, PendingSend.State.Sending), pane.sends.all.map { it.state })
        store.accept(decode("""{"t":"convo.heard","pane":"$PANE","text":"again"}"""))
        assertEquals(listOf(PendingSend.State.Delivered, PendingSend.State.Delivered), pane.sends.all.map { it.state })
    }

    @Test
    fun aHookQuestionsIdRidesOnItsAnswersAndAnOlderNodesDoesNot() {
        val asked = decode(
            """{"t":"pending","pane":"$PANE","id":"q7","question":"Run?","source":"hook","options":[{"key":"1","label":"Yes"}]}""",
        ) as ServerMsg.Pending
        assertEquals("q7", asked.id)
        val old = decode("""{"t":"pending","pane":"$PANE","question":"Run?","source":"screen","options":[]}""") as ServerMsg.Pending
        assertNull(old.id)

        assertEquals("""{"t":"answer","pane":"$PANE","key":"1","id":"q7"}""", Wire.encode(ClientMsg.Answer(PANE, "1", "q7")))
        assertEquals("""{"t":"answer","pane":"$PANE","key":"1"}""", Wire.encode(ClientMsg.Answer(PANE, "1")))
        assertEquals("""{"t":"answer.submit","pane":"$PANE","id":"q7"}""", Wire.encode(ClientMsg.AnswerSubmit(PANE, "q7")))
        assertEquals("""{"t":"answer.submit","pane":"$PANE"}""", Wire.encode(ClientMsg.AnswerSubmit(PANE)))
    }

    @Test
    fun anOptionAnsweredInWordsSaysSoAndItsAnswerCarriesThem() {
        val asked = decode(
            """{"t":"pending","pane":"$PANE","id":"q9","question":"Approve Claude's plan?","header":"Plan","source":"hook",
               "options":[{"key":"1","label":"Yes, auto-accept edits"},{"key":"3","label":"No, tell Claude what to change","text":true}]}""",
        ) as ServerMsg.Pending
        assertEquals(listOf(false, true), asked.options.map { it.text })

        assertEquals(
            """{"t":"answer","pane":"$PANE","key":"3","id":"q9","text":"split the migration"}""",
            Wire.encode(ClientMsg.Answer(PANE, "3", "q9", "split the migration")),
        )
        assertEquals("""{"t":"answer","pane":"$PANE","key":"3","id":"q9"}""", Wire.encode(ClientMsg.Answer(PANE, "3", "q9", null)))
    }

    @Test
    fun aHookQuestionWithNoOptionsIsAnsweredOnlyAtTheTerminal() {
        val form = decode("""{"t":"pending","pane":"$PANE","id":"q1","question":"Fill the form","source":"hook","options":[]}""") as ServerMsg.Pending
        assertTrue(form.terminalOnly)
        val screen = decode("""{"t":"pending","pane":"$PANE","question":"Proceed?","source":"screen","options":[]}""") as ServerMsg.Pending
        assertFalse(screen.terminalOnly)
    }

    // A slash command is heard, and its record never matches what was sent, so the reply that
    // follows it is what retires it. An answer already there when it was heard, or one still being
    // painted, is not that reply.
    @Test
    fun aDeliveredSendGoesWhenAReplyNewerThanItArrives() {
        val store = KamprStore()
        val pane = store.pane(PANE)
        fun reply(id: String) = store.accept(ServerMsg.ConvoTurn(PANE, listOf(Turn(id, "assistant", blocks = listOf(Block.Md("ok $id"))))))
        reply("a0")
        pane.sends.sent("/compact", pane.turns, emptyList(), delivered = true, at = 0.0)
        reply("a1")
        store.accept(decode("""{"t":"convo.heard","pane":"$PANE","text":"/compact"}"""))
        assertEquals(PendingSend.State.Delivered, pane.sends.all.single().state)

        reply("a0")
        store.accept(ServerMsg.ConvoTurn(PANE, listOf(Turn("live", "assistant", blocks = listOf(Block.Md("work"))))))
        assertEquals(1, pane.sends.all.size, "a reply from before it was heard, or a live one, retired it")
        pane.sends.heard(listOf(Turn("older", "assistant")) + pane.turns, emptyList())
        assertEquals(1, pane.sends.all.size, "a page of older turns was taken for the reply to it")

        reply("a2")
        assertTrue(pane.sends.all.isEmpty(), "a delivered send outlived the reply to it")
    }

    @Test
    fun onlyADeliveredSendExpires() {
        val pane = KamprStore().pane(PANE)
        val waiting = pane.sends.sent("one", emptyList(), emptyList(), delivered = true, at = 0.0)
        pane.sends.expired(waiting.id)
        assertEquals(1, pane.sends.all.size, "a send nobody has heard yet was dropped")
        pane.sends.acknowledged("one", emptyList())
        pane.sends.expired(waiting.id)
        assertTrue(pane.sends.all.isEmpty())
    }
}
