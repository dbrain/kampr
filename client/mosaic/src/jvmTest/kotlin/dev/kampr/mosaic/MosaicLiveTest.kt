package dev.kampr.mosaic

import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.TerminalSurfaces
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The real client against a real mesh, the way LiveNodeTest does it: point KAMPR_URL and
// KAMPR_TOKEN at a running hub with a peer enrolled. With neither set the test skips, so the
// suite stays green on a machine with no herd.
private val URL: String? = System.getenv("KAMPR_URL")
private val TOKEN: String? = System.getenv("KAMPR_TOKEN")

// Set once the peer node has been killed, so the second half can prove the degrade.
private val PEER_DOWN: Boolean = System.getenv("KAMPR_PEER_DOWN") != null

private class LiveIo(private val connection: KamprConnection, private val store: KamprStore) : PaneIo {
    override fun send(msg: ClientMsg) = connection.send(msg)
    override fun prefs(paneId: String): PanePrefs = store.prefsFor(paneId)
    override fun info(paneId: String): PaneInfo? = store.paneInfo(paneId)
}

private class Live {
    val scope = CoroutineScope(SupervisorJob())
    val store = KamprStore()
    val connection = KamprConnection(scope, store)
    val mosaic = MosaicState(MemoryPrefs(), connection)

    fun open() = connection.connect(Endpoint(URL!!, TOKEN!!))

    fun close() {
        connection.disconnect()
        scope.cancel()
    }
}

class MosaicLiveTest {
    @Test
    fun aMosaicShowsPanesFromTwoRealNodesAndTypesIntoTheFocusedOne() {
        if (URL == null || TOKEN == null || PEER_DOWN) return
        val live = Live()
        try {
            runBlocking {
                live.open()
                await("hello") { live.store.hello.value }
                assertEquals(ConnectionStatus.Live("full"), live.store.status.value)

                val herd = await("a herd with a peer") {
                    live.store.herd.value.takeIf { it.known && it.nodes.any { n -> n.kind == "peer" && n.online } }
                }
                val local = pick(herd, "local")
                val peer = pick(herd, "peer")
                assertTrue(local.nodeId != peer.nodeId, "two panes from one node prove nothing")

                live.mosaic.attach()
                live.mosaic.add(local.id)
                live.mosaic.add(peer.id)
                live.mosaic.focus(local.id)
                assertEquals(2, live.mosaic.observers)

                val localPane = live.store.pane(local.id)
                val peerPane = live.store.pane(peer.id)
                await("the local grid") { Unit.takeIf { localPane.painted } }
                await("the peer grid across the mesh") { Unit.takeIf { peerPane.painted } }

                // Input goes to the focused cell only. Both cells are live streams; only one of
                // them has a writable PaneIo behind it, so only one of them can have typed this.
                val marker = "kampr-mosaic-focus-${System.currentTimeMillis() % 100000}"
                live.connection.send(ClientMsg.InputText(local.id, "echo $marker\r"))
                await("the marker in the focused pane") { Unit.takeIf { grid(localPane).contains(marker) } }
                assertTrue(!grid(peerPane).contains(marker), "input reached an unfocused cell")

                val peerMarker = "kampr-mosaic-peer-${System.currentTimeMillis() % 100000}"
                live.mosaic.focus(peer.id)
                live.connection.send(ClientMsg.InputText(peer.id, "echo $peerMarker\r"))
                await("the marker on the peer, through the hub") {
                    Unit.takeIf { grid(peerPane).contains(peerMarker) }
                }

                live.mosaic.focus(local.id)
                delay(600)
                render(live, "live-mosaic.png")
            }
        } finally {
            live.close()
        }
    }

    @Test
    fun killingThePeerDegradesOnlyItsOwnCell() {
        if (URL == null || TOKEN == null || !PEER_DOWN) return
        val live = Live()
        try {
            runBlocking {
                live.open()
                await("hello") { live.store.hello.value }
                val herd = await("the peer marked down") {
                    live.store.herd.value.takeIf {
                        it.known && it.nodes.any { n -> n.kind == "peer" && !n.online && n.detail != null }
                    }
                }
                val local = pick(herd, "local")
                val peer = pick(herd, "peer")
                assertTrue(
                    herd.panes.any { it.nodeId == peer.nodeId },
                    "a dropped peer's panes must stay listed, or the mosaic empties out from under the operator",
                )

                live.mosaic.attach()
                live.mosaic.add(local.id)
                live.mosaic.add(peer.id)
                live.mosaic.focus(local.id)
                live.mosaic.reconcile(herd)
                assertEquals(2, live.mosaic.panes.size, "the dead peer's cell must survive")

                val localPane = live.store.pane(local.id)
                await("the surviving local grid") { Unit.takeIf { localPane.painted } }

                val marker = "kampr-mosaic-alive-${System.currentTimeMillis() % 100000}"
                live.connection.send(ClientMsg.InputText(local.id, "echo $marker\r"))
                await("the surviving cell still taking input") {
                    Unit.takeIf { grid(localPane).contains(marker) }
                }

                delay(400)
                render(live, "live-mosaic-peer-down.png")
            }
        } finally {
            live.close()
        }
    }

    private fun render(live: Live, name: String) {
        val io = LiveIo(live.connection, live.store)
        renderArtboard(DESKTOP.first, DESKTOP.second, SoftTheme, TypeScale.Desk, File(OUT, name)) {
            androidx.compose.runtime.CompositionLocalProvider(LocalPaneIo provides io) {
                MosaicScreen(
                    store = live.store,
                    mosaic = live.mosaic,
                    herd = live.store.herd.value,
                    connectionStatus = live.store.status.value,
                    build = live.store.hello.value?.build,
                    surfaces = TerminalSurfaces(),
                    onHerd = {},
                    onAdd = {},
                )
            }
        }
        assertTrue(File(OUT, name).length() > 0, "$name rendered nothing")
    }
}

private fun pick(herd: Herd, kind: String): PaneInfo {
    val ids = herd.nodes.filter { it.kind == kind }.map { it.id }.toSet()
    return herd.panes.firstOrNull { it.nodeId in ids } ?: error("no $kind pane in ${herd.nodes.map { it.name }}")
}

private fun grid(pane: PaneState): String = buildString {
    for (row in 0 until pane.cells.rows) {
        for (col in 0 until pane.cells.cols) append(pane.cells.charAt(col, row))
        append('\n')
    }
}

private suspend fun <T : Any> await(what: String, poll: () -> T?): T {
    repeat(600) {
        poll()?.let { return it }
        delay(50)
    }
    error("never saw $what")
}
