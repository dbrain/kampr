package dev.kampr.shared

import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.SplitDirection
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

// The real client against a real node: point KAMPR_URL and KAMPR_TOKEN at a running `kampr serve`
// and its device token. With neither set the test skips, so the suite stays green on a machine
// with no herd.
private val URL: String? = System.getenv("KAMPR_URL")
private val TOKEN: String? = System.getenv("KAMPR_TOKEN")

class LiveNodeTest {
    @Test
    fun theClientDrivesARealHerdAndLearnsAboutItFromThePatch() {
        val url = URL ?: return
        val token = TOKEN ?: return
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(scope, store)
        try {
            runBlocking {
                connection.connect(Endpoint(url, token))
                val hello = await("hello") { store.hello.value }
                assertTrue(hello.caps.manage, "this node does not offer manage")
                assertTrue(store.canManage)
                assertEquals(ConnectionStatus.Live("full"), store.status.value)

                // Nothing in this test asked for `caps`. The connection does it on `hello`,
                // which is the whole point: agent kinds were dead on both ends without it.
                val caps = await("caps") { store.capsFor(hello.nodeId) }
                assertTrue(caps.agentKinds.contains("claude"), "${caps.agentKinds}")

                val node = await("herd") { store.herd.value.takeIf { it.known }?.nodes?.firstOrNull() }.id
                val before = store.herd.value.panes.map { it.id }.toSet()

                connection.manage(ManageOp.WorkspaceCreate(node, "kampr-live", "/tmp", mapOf("KAMPR_LIVE" to "1")))
                val ack = await("managed") { store.managed.value?.takeIf { it.op == "workspace.create" } }
                assertTrue(ack.ok, "${ack.code} ${ack.message}")
                val workspace = assertNotNull(ack.id)

                // The ack carries an id and nothing else; the pane arrives only when the node
                // says so, in a `herd.patch`, on this same connection.
                val fresh = await("herd.patch") {
                    store.herd.value.panes.firstOrNull { it.id !in before && it.workspaceId == workspace }
                }
                assertEquals(workspace, fresh.workspaceId)
                assertNotNull(fresh.tabId, "a tab a client can address")

                connection.manage(ManageOp.PaneSplit(fresh.id, SplitDirection.Right, 0.35))
                await("split") { store.managed.value?.takeIf { it.op == "pane.split" && it.ok } }
                val second = await("the split pane") {
                    store.herd.value.panes.firstOrNull { it.workspaceId == workspace && it.id != fresh.id }
                }

                val agent = startAgent(connection, store, second)
                assertEquals("claude", agent.agent)

                connection.manage(ManageOp.Rename(fresh.id, "live"))
                await("rename") { store.managed.value?.takeIf { it.op == "rename" && it.ok } }
                await("the renamed pane") {
                    store.herd.value.panes.firstOrNull { it.id == fresh.id && it.label == "live" }
                }
                connection.manage(ManageOp.Rename(fresh.id, null))
                await("the cleared label") {
                    store.managed.value?.takeIf { it.op == "rename" && it.ok }
                        ?.takeIf { store.herd.value.panes.none { p -> p.id == fresh.id && p.label == "live" } }
                }

                connection.manage(ManageOp.Close(workspace))
                await("close") { store.managed.value?.takeIf { it.op == "close" && it.ok } }
                await("the workspace going away") {
                    Unit.takeIf { store.herd.value.panes.none { it.workspaceId == workspace } }
                }
            }
        } finally {
            connection.disconnect()
            scope.cancel()
        }
    }

    // A pane herdr has only just created is not yet an available shell, so this is the one op
    // that waits for the thing it acts on rather than for its own answer.
    private suspend fun startAgent(
        connection: KamprConnection,
        store: KamprStore,
        pane: PaneInfo,
    ): PaneInfo {
        repeat(20) {
            store.clearManaged()
            connection.manage(ManageOp.AgentStart(pane.id, "claude", "live"))
            val ack = await("agent.start") { store.managed.value?.takeIf { it.op == "agent.start" } }
            if (ack.ok) {
                return await("the agent") {
                    store.herd.value.panes.firstOrNull { it.id == pane.id && it.agent != null }
                }
            }
            delay(500)
        }
        error("agent.start never succeeded")
    }
}

private suspend fun <T : Any> await(what: String, poll: () -> T?): T {
    repeat(600) {
        poll()?.let { return it }
        delay(50)
    }
    error("never saw $what")
}
