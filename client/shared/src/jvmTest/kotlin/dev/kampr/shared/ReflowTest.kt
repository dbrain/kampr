package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.reflowCols
import dev.kampr.shared.model.reflowNote
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

private val SHELL = PaneInfo("01JNODE/n:01ja", "01JNODE", provider = "native", sessionId = "01ja", cols = 120)
private val CLAUDE = SHELL.copy(agent = "claude")
private val HERDR = PaneInfo("01JNODE/w1:p1", "01JNODE", cols = 120)

class ReflowTest {
    @Test
    fun aWatchAsksForAReflowOnlyWhenItWantsOne() {
        assertEquals(
            """{"t":"watch","pane":"p","scrollback":true,"conversation":true}""",
            Wire.encode(ClientMsg.Watch("p")),
        )
        assertEquals(
            """{"t":"watch","pane":"p","scrollback":true,"conversation":true,"reflow":{"cols":40}}""",
            Wire.encode(ClientMsg.Watch("p", reflow = 40)),
        )
    }

    @Test
    fun aResetSaysWhichGridItIsAndAnOlderNodeSaysNothing() {
        val frame = { extra: String ->
            """{"t":"grid.reset","pane":"p","cols":40,"rows":24,"rows_data":[],"cursor":{"col":0,"row":0,"visible":true},"links":[]$extra}"""
        }
        for ((extra, want) in listOf("" to null, ""","reflow":"on"""" to "on", ""","reflow":"harness"""" to "harness")) {
            val reset = Wire.decode(frame(extra)) as ServerMsg.GridReset
            assertEquals(want, reset.reflow, extra)
            val store = KamprStore()
            store.accept(reset)
            assertEquals(want, store.pane("p").reflow, extra)
        }
    }

    @Test
    fun aPhoneReLaysANarrowerShellAndNothingElseUnlessAsked() {
        val on = PanePrefs(mapOf("reflow" to "on"))
        val off = PanePrefs(mapOf("reflow" to "off"))
        data class Case(val info: PaneInfo?, val view: Int, val desk: Boolean, val mosaic: Boolean, val prefs: PanePrefs, val want: Int?)
        val cases = listOf(
            Case(SHELL, 40, false, false, PanePrefs(), 40),
            Case(SHELL, 130, false, false, PanePrefs(), null),
            Case(CLAUDE, 40, false, false, PanePrefs(), null),
            Case(HERDR, 40, false, false, PanePrefs(), null),
            Case(HERDR, 40, false, false, on, null),
            Case(SHELL, 40, true, false, PanePrefs(), null),
            Case(SHELL, 40, false, true, PanePrefs(), null),
            Case(SHELL, 40, false, true, on, null),
            Case(SHELL, 40, false, false, off, null),
            Case(SHELL, 90, true, false, on, 90),
            Case(CLAUDE, 40, false, false, on, 40),
            Case(null, 40, false, false, on, null),
            Case(SHELL.copy(cols = null), 40, false, false, PanePrefs(), null),
        )
        for (c in cases) assertEquals(c.want, reflowCols(c.info, c.view, c.desk, c.mosaic, c.prefs.reflow), "$c")
    }

    @Test
    fun theNoteSaysWhatTheGridIs() {
        assertEquals("Re-laid at 40 columns for this screen; the pane is still 120.", reflowNote("on", 40, 120))
        assertEquals(
            "Showing the pane's own 120 columns: a full-screen program has the screen.",
            reflowNote("alternate", 40, 120),
        )
        assertEquals("Showing the pane's own 120 columns: an agent lays out its own lines.", reflowNote("harness", 40, 120))
        assertEquals("Showing the pane's own 120 columns: this pane can't be re-laid.", reflowNote("unsupported", 40, 120))
        assertEquals("Showing the pane's own 120 columns: this node can't re-lay a pane.", reflowNote(null, 40, 120))
    }

    @Test
    fun aReflowIsSentWhenItChangesAndAgainOnEveryReconnect() {
        val node = StubNode()
        node.start()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(scope, store)
        val watches = { node.received.filter { it.contains("\"watch\"") } }
        try {
            runBlocking {
                connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
                until("hello") { store.hello.value != null }
                connection.watch(SHELL.id)
                connection.reflow(SHELL.id, 40)
                connection.reflow(SHELL.id, 40)
                until("the reflowed watch") { watches().any { it.contains("\"reflow\":{\"cols\":40}") } }
                connection.reflow(SHELL.id, null)
                until("the plain watch again") { watches().size >= 3 }
                assertEquals(3, watches().size, "${watches()}")
                assertEquals(false, watches().last().contains("reflow"), watches().last())

                connection.reflow(SHELL.id, 52)
                until("the new width") { watches().last().contains("\"cols\":52") }
                node.stop()
                until("the socket to drop") { store.status.value !is dev.kampr.shared.model.ConnectionStatus.Live }
                val before = watches().size
                node.start()
                until("the re-watch", timeoutMs = 30_000) { watches().size > before }
                assertEquals(true, watches().last().contains("\"reflow\":{\"cols\":52}"), watches().last())
            }
        } finally {
            connection.disconnect()
            scope.cancel()
            node.stop()
        }
    }
}
