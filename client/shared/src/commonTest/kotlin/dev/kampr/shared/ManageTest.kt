package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import dev.kampr.shared.wire.workspaceIdOf
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w3:p2"

private fun hello(role: String = "full", manage: Boolean = true) = Wire.decode(
    """{"t":"hello","protocol":1,"node_id":"01JNODE","node_name":"comingclean","build":"0.1.0",
       "role":"$role","caps":{"push":false,"scrollback":true,"conversation":true,"manage":$manage}}"""
)!!

class ManageTest {
    @Test
    fun capsIsAskedFor() {
        assertEquals("""{"t":"caps"}""", Wire.encode(ClientMsg.RequestCaps))
    }

    @Test
    fun theNodesCapsAnswerCarriesKindsAndSessions() {
        val msg = Wire.decode(
            """{"t":"caps","node":"01JNODE","agent_kinds":["claude","codex","gemini"],
               "sessions":[{"name":"default","running":true},{"name":"agents","running":false}]}"""
        ) as ServerMsg.NodeCaps
        assertEquals(listOf("claude", "codex", "gemini"), msg.agentKinds)
        assertEquals("agents", msg.sessions[1].name)
        assertFalse(msg.sessions[1].running)
    }

    @Test
    fun aRefusedOpIsAnAckWithACodeAndNotOnlyAnErrorFrame() {
        val ok = Wire.decode("""{"t":"managed","op":"workspace.create","ok":true,"id":"01JNODE/w9"}""") as ServerMsg.Managed
        assertTrue(ok.ok)
        assertEquals("01JNODE/w9", ok.id)
        assertNull(ok.code)

        val refused = Wire.decode(
            """{"t":"managed","op":"pane.split","ok":false,"code":"not_writer","message":"read-only"}"""
        ) as ServerMsg.Managed
        assertFalse(refused.ok)
        assertEquals("not_writer", refused.code)
        assertEquals("read-only", refused.message)
    }

    // error.code is an open string, so an unknown one still has to reach the operator by message.
    @Test
    fun anUnknownCodeStillCarriesItsMessage() {
        val refused = Wire.decode(
            """{"t":"managed","op":"pane.split","ok":false,"code":"quota_exhausted","message":"too many panes"}"""
        ) as ServerMsg.Managed
        assertEquals("too many panes", refused.message)
        assertEquals("quota_exhausted", refused.code)
    }

    @Test
    fun aLayoutExportComesBackOnItsAck() {
        val ack = Wire.decode(
            """{"t":"managed","op":"layout.export","ok":true,
               "layout":{"tab_id":"w3:t1","root":{"type":"split","direction":"right"}}}"""
        ) as ServerMsg.Managed
        val layout = ack.layout!!
        assertEquals("w3:t1", layout["tab_id"].toString().trim('"'))
        // Round-trips straight back into layout.apply: the client never reads inside the tree.
        val apply = Wire.encode(ClientMsg.Manage(dev.kampr.shared.wire.ManageOp.LayoutApply("01JNODE/w3:t1", layout)))
        assertTrue(apply.contains(""""root":{"type":"split","direction":"right"}"""), apply)
    }

    @Test
    fun aPaneCarriesTheIdsItsContainersAreAddressedBy() {
        val herd = Wire.decode(
            """{"t":"herd","nodes":[{"id":"01JNODE","name":"comingclean","kind":"local"}],
               "panes":[{"id":"$PANE","node_id":"01JNODE","workspace_id":"01JNODE/w3",
                         "tab_id":"01JNODE/w3:t1","workspace":"kampr","tab":"1","cols":74,"rows":30}]}"""
        ) as ServerMsg.Herd
        val pane = herd.panes.single()
        assertEquals("01JNODE/w3", pane.workspaceId)
        assertEquals("01JNODE/w3:t1", pane.tabId)
        // A tab id is underivable from a pane id; a workspace id is not, which is the fallback
        // for a node too old to send them.
        assertEquals("01JNODE/w3", workspaceIdOf(pane.id))
    }

    @Test
    fun manageIsHiddenWhenTheNodeOrTheRoleSaysNo() {
        val store = KamprStore()
        assertFalse(store.canManage)
        store.accept(hello())
        assertTrue(store.canManage)
        store.accept(hello(role = "readonly"))
        assertFalse(store.canManage)
        store.accept(hello(manage = false))
        assertFalse(store.canManage)
    }

    @Test
    fun capsFallBackToTheOneAnswerTheNodeGives() {
        val store = KamprStore()
        assertNull(store.capsFor("01JNODE"))
        store.accept(Wire.decode("""{"t":"caps","node":"01JNODE","agent_kinds":["claude"]}""")!!)
        assertEquals(listOf("claude"), store.capsFor("01JNODE")?.agentKinds)
        // A named session is its own node id; the node still answers caps under its own.
        assertEquals(listOf("claude"), store.capsFor("01JNODE.agents")?.agentKinds)
    }

    // The node is authoritative: an ack changes nothing in the herd, and the patch that follows
    // is what puts the new workspace on screen.
    @Test
    fun anAckNeverMutatesTheHerdAndThePatchDoes() {
        val store = KamprStore()
        store.accept(hello())
        store.accept(Wire.decode("""{"t":"herd","nodes":[{"id":"01JNODE","kind":"local"}],"panes":[]}""")!!)
        store.accept(Wire.decode("""{"t":"managed","op":"workspace.create","ok":true,"id":"01JNODE/w9"}""")!!)
        assertTrue(store.herd.value.panes.isEmpty())
        assertEquals("01JNODE/w9", store.managed.value?.id)

        store.accept(
            Wire.decode(
                """{"t":"herd.patch","added":{"panes":[{"id":"01JNODE/w9:p1","node_id":"01JNODE",
                   "workspace_id":"01JNODE/w9","tab_id":"01JNODE/w9:t1","workspace":"probe",
                   "cols":80,"rows":24}]}}"""
            )!!
        )
        assertEquals(listOf("01JNODE/w9:p1"), store.herd.value.panes.map { it.id })
        assertEquals("01JNODE/w9:t1", store.herd.value.panes.single().tabId)
    }
}
