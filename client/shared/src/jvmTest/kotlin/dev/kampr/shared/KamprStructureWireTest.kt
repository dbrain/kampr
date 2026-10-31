package dev.kampr.shared

import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.PROVIDER_HERDR
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

// Workspaces, tabs, agents and worktrees are Kampr's first and herdr's only when asked for by name:
// an op with no `provider` and no herdr `at` is made as Kampr sessions, and the "with herdr" half of
// the sheet says `provider: "herdr"` on every op it sends.
class KamprStructureWireTest {
    @Test
    fun theStructureOpsEncodeToTheContractShape() {
        val node = "01JNODE"
        val cases = listOf(
            ManageOp.WorkspaceCreate(node, label = "api", cwd = "/srv/api", command = "make dev") to
                """{"t":"manage","op":"workspace.create","node":"$node","label":"api","cwd":"/srv/api","command":"make dev"}""",
            ManageOp.WorkspaceCreate(node, label = "api", provider = PROVIDER_HERDR) to
                """{"t":"manage","op":"workspace.create","node":"$node","label":"api","provider":"herdr"}""",
            ManageOp.TabCreate("$node/kw:api") to """{"t":"manage","op":"tab.create","at":"$node/kw:api"}""",
            ManageOp.TabCreate("$node/w3", provider = PROVIDER_HERDR) to
                """{"t":"manage","op":"tab.create","at":"$node/w3","provider":"herdr"}""",
            ManageOp.AgentStart(kind = "claude", node = node) to
                """{"t":"manage","op":"agent.start","kind":"claude","node":"$node"}""",
            ManageOp.AgentStart(kind = "codex", at = "$node/n:01ja", args = listOf("--full-auto")) to
                """{"t":"manage","op":"agent.start","at":"$node/n:01ja","kind":"codex","args":["--full-auto"]}""",
            ManageOp.AgentStart(kind = "claude", node = node, workspace = "$node/kw:api", cwd = "/srv/api") to
                """{"t":"manage","op":"agent.start","kind":"claude","node":"$node","workspace":"$node/kw:api","cwd":"/srv/api"}""",
            ManageOp.AgentStart(kind = "claude", at = "$node/w3:p2", provider = PROVIDER_HERDR) to
                """{"t":"manage","op":"agent.start","at":"$node/w3:p2","kind":"claude","provider":"herdr"}""",
            ManageOp.WorktreeCreate(node, "feat/x", base = "main", cwd = "/srv/api") to
                """{"t":"manage","op":"worktree.create","node":"$node","branch":"feat/x","base":"main","cwd":"/srv/api"}""",
            ManageOp.WorktreeCreate(node, "feat/x", provider = PROVIDER_HERDR) to
                """{"t":"manage","op":"worktree.create","node":"$node","branch":"feat/x","provider":"herdr"}""",
            ManageOp.WorktreeOpen(node, "/srv/api.worktrees/feat-x", provider = PROVIDER_HERDR) to
                """{"t":"manage","op":"worktree.open","node":"$node","path":"/srv/api.worktrees/feat-x","provider":"herdr"}""",
            ManageOp.NativeCreate(node, workspace = "$node/kw:api") to
                """{"t":"manage","op":"native.create","node":"$node","workspace":"$node/kw:api"}""",
        )
        for ((op, json) in cases) {
            assertEquals(json, Wire.encode(ClientMsg.Manage(op)), "$op")
        }
    }

    @Test
    fun capsSayWhichAgentsANodeCanRunWithoutHerdr() {
        val caps = Wire.decode("""{"t":"caps","node":"n","agent_kinds":["claude","amp"],"sessions":[],"native_agents":["claude","codex"]}""")
        assertEquals(listOf("claude", "codex"), (caps as ServerMsg.NodeCaps).nativeAgents)
        val older = Wire.decode("""{"t":"caps","node":"n","agent_kinds":["claude"],"sessions":[]}""")
        assertNull((older as ServerMsg.NodeCaps).nativeAgents, "a node too old to say offers none")
    }
}
