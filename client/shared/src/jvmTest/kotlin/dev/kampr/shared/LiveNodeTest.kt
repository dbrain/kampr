package dev.kampr.shared

import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.AuthApi
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.net.createHttpClient
import dev.kampr.shared.wire.ClientMsg
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
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertTrue
import kotlin.test.fail

// The real client against a real node: point KAMPR_URL and KAMPR_TOKEN at a running `kampr serve`
// and its device token.
private val URL: String? = System.getenv("KAMPR_URL")
private val TOKEN: String? = System.getenv("KAMPR_TOKEN")

// A silent skip is how this suite came to have never run once, and why a client that called four
// `/auth/*` paths no node has ever routed stayed green for a whole phase. A skip says so on
// stderr; a run that was *meant* to be live fails outright rather than passing empty.
private val REQUIRED: Boolean = System.getenv("KAMPR_LIVE") != null

private fun live(): Endpoint? {
    val url = URL
    val token = TOKEN
    if (!url.isNullOrBlank() && !token.isNullOrBlank()) return Endpoint(url, token)
    val why = "LiveNodeTest SKIPPED — KAMPR_URL/KAMPR_TOKEN unset, no real node was exercised"
    if (REQUIRED) fail("$why, and KAMPR_LIVE demanded one")
    System.err.println("\n${"!".repeat(78)}\n  $why\n${"!".repeat(78)}\n")
    return null
}

class LiveNodeTest {
    @Test
    fun theClientDrivesARealHerdAndLearnsAboutItFromThePatch() {
        val target = live() ?: return
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(scope, store)
        try {
            runBlocking {
                connection.connect(target)
                val hello = await("hello") { store.hello.value }
                assertTrue(hello.caps.manage, "this node does not offer manage")
                assertTrue(store.canManage)
                assertEquals(ConnectionStatus.Live("full"), store.status.value)

                // Nothing in this test asked for `caps`. The connection does it on `hello`,
                // which is the whole point: agent kinds were dead on both ends without it.
                val caps = await("caps") { store.capsFor(hello.nodeId) }
                assertTrue(caps.agentKinds.contains("claude"), "${caps.agentKinds}")

                // The node this socket belongs to, not whichever session sorted first: one host
                // runs a herdr server per named session and every one of them is its own node.
                val node = await("herd") {
                    store.herd.value.takeIf { it.known }?.nodes?.firstOrNull { it.kind == "local" }
                }.id
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

    // Every HTTP path the client has: the setup wizard's address and version, the device list, a
    // pairing code a second device can actually redeem, and revocation. None of it was covered
    // here before, which is exactly why three of the four paths pointed at routes the node has
    // never had.
    @Test
    fun theClientReadsTheAuthSurfaceOfARealNode() {
        val target = live() ?: return
        runBlocking {
            val client = createHttpClient()
            try {
                val api = AuthApi(client, target)

                val status = assertNotNull(api.status(), "the setup wizard has no status to show")
                assertTrue(status.address.startsWith("http"), "address was '${status.address}'")
                assertNotNull(status.version, "the wizard has no build to show")

                val devices = api.devices()
                assertTrue(devices.isNotEmpty(), "this very connection is a paired device")
                assertTrue(devices.all { it.name.isNotBlank() }, "${devices.map { it.name }}")
                assertTrue(devices.any { it.role == "full" }, "${devices.map { it.role }}")
                assertEquals(devices.count { it.active }, status.devices)

                // A code this device asked for is armed by construction, so it redeems as it
                // stands — which is the only thing that makes the browser wizard actionable.
                val code = assertNotNull(api.pairingCode(), "the wizard cannot offer a pairing code")
                val enrolled = assertNotNull(api.pair(code, "kampr-live-test"), "a fresh code did not redeem")
                assertTrue(enrolled.token.isNotBlank())
                val added = await("the new device in the list") {
                    api.devices().firstOrNull { it.id == enrolled.deviceId }
                }
                assertEquals("kampr-live-test", added.name, "the client never sent a device name")

                // §3.2 of the threat model names this as the whole stolen-phone mitigation.
                assertTrue(api.revoke(added.id), "revoke was refused")
                await("the device losing access") {
                    Unit.takeIf { api.devices().none { it.id == added.id && it.active } }
                }
                assertFalse(api.revoke("no-such-device-at-all"), "a refused revoke reported success")
            } finally {
                client.close()
            }
        }
    }

    // "Remembered zoom levels", asked for twice. Two one-key writes on one connection, and both
    // have to survive the next one — the node pushes them unasked at `hello`, because a client
    // that has to ask has already painted the pane at the wrong size.
    @Test
    fun preferencesWrittenOnOneConnectionComeBackOnTheNext() {
        val target = live() ?: return
        val scope = CoroutineScope(SupervisorJob())
        try {
            runBlocking {
                val first = KamprConnection(scope, KamprStore())
                first.connect(target)
                val local = await("the local node") {
                    first.store.herd.value.takeIf { it.known }?.nodes?.firstOrNull { it.kind == "local" }
                }
                first.manage(ManageOp.WorkspaceCreate(local.id, "kampr-prefs", "/tmp"))
                val workspace = assertNotNull(
                    await("managed") { first.store.managed.value?.takeIf { it.op == "workspace.create" } }.id,
                )
                val pane = await("a pane") {
                    first.store.herd.value.panes.firstOrNull { it.workspaceId == workspace }
                }.id
                first.send(ClientMsg.SetPrefs(pane, mapOf("zoom" to "1.62")))
                await("the zoom write") { first.store.prefsFor(pane).zoom }
                // The second write names one key. It must not thereby forget the first.
                first.send(ClientMsg.SetPrefs(pane, mapOf("view" to "terminal")))
                await("the view write") { first.store.prefsFor(pane).view }
                assertEquals(1.62f, first.store.prefsFor(pane).zoom, "a one-key write erased the zoom")
                first.disconnect()

                val second = KamprConnection(scope, KamprStore())
                second.connect(target)
                await("hello") { second.store.hello.value }
                val restored = await("prefs pushed at hello") {
                    second.store.prefsFor(pane).takeIf { it.zoom != null }
                }
                assertEquals(1.62f, restored.zoom)
                assertEquals("terminal", restored.view)
                second.manage(ManageOp.Close(workspace))
                await("close") { second.store.managed.value?.takeIf { it.op == "close" && it.ok } }
                second.disconnect()
            }
        } finally {
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

private suspend fun <T : Any> await(what: String, poll: suspend () -> T?): T {
    repeat(600) {
        poll()?.let { return it }
        delay(50)
    }
    error("never saw $what")
}
