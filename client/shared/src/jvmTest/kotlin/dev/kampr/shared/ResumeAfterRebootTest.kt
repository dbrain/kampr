package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.push.NoPush
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppManage
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.MenuAnchor
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.wire.ResumeRecord
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

private const val HUB = "01JHUB"
private const val PEER = "01JPEER"

private val NVIM = ResumeRecord("01k8/74877", "work", listOf("/usr/bin/nvim", "-S", "/home/u/s.vim"), "/home/u/dev")
private val SNEAKY = ResumeRecord("01k9/1", null, listOf("sh", "-c", "echo hi; \$(reboot)"), null)

private const val HERD_JSON = """{"t":"herd","nodes":[
  {"id":"$HUB","name":"hub","kind":"local","online":true,"native":true,"resume":true},
  {"id":"$PEER","name":"peer","kind":"peer","online":true,"native":true,"resume":true}],"panes":[]}"""

private val HERD: Herd = (Wire.decode(HERD_JSON) as ServerMsg.Herd).let { Herd(it.nodes, it.panes, known = true) }

private class Offering(private val records: Map<String, List<ResumeRecord>>, override val enabled: Boolean = true) : ManageIo {
    val resumed = mutableListOf<Pair<String, String>>()
    val dismissed = mutableListOf<Pair<String, String>>()
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
    override fun openMenu(paneId: String, at: MenuAnchor?) = Unit
    override fun resumable(nodeId: String): List<ResumeRecord> = records[nodeId].orEmpty()
    override fun resume(nodeId: String, record: ResumeRecord) {
        resumed += nodeId to record.id
    }

    override fun dismissResume(nodeId: String, record: ResumeRecord) {
        dismissed += nodeId to record.id
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.count(label: String): Int = onAllNodesWithContentDescription(label, substring = true).fetchSemanticsNodes().size

// docs/25, under the default `ask`: what a boot left on a machine is listed under that machine,
// each with its program, where it ran and exactly what would run, and nothing runs until a person
// taps it.
@OptIn(ExperimentalTestApi::class)
class ResumeAfterRebootTest {
    @Composable
    private fun Portrait(manage: ManageIo) {
        CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalManage provides manage) {
            Box(Modifier.size(420.dp, 3000.dp)) {
                HerdPortrait(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null)
            }
        }
    }

    @Test
    fun aMachineWithRecordsListsThemAndOneWithoutShowsNothing() = runComposeUiTest {
        setContent { Portrait(Offering(mapOf(HUB to listOf(NVIM, SNEAKY)))) }
        assertEquals(1, count("Resume after reboot on hub"))
        assertEquals(0, count("Resume after reboot on peer"))
        assertEquals(1, count("nvim, in ~/dev, runs /usr/bin/nvim -S /home/u/s.vim"))
        assertEquals(1, count("sh, runs sh -c 'echo hi; \$(reboot)'"))
    }

    @Test
    fun resumeAndDismissAskTheMachineTheRecordIsOn() = runComposeUiTest {
        val manage = Offering(mapOf(HUB to listOf(NVIM)))
        setContent { Portrait(manage) }
        onNodeWithContentDescription("Resume nvim").performClick()
        onNodeWithContentDescription("Dismiss nvim").performClick()
        assertEquals(listOf(HUB to NVIM.id), manage.resumed)
        assertEquals(listOf(HUB to NVIM.id), manage.dismissed)
    }

    @Test
    fun aDeviceThatCannotManageIsNotOfferedAnything() = runComposeUiTest {
        setContent { Portrait(Offering(mapOf(HUB to listOf(NVIM)), enabled = false)) }
        assertEquals(0, count("Resume after reboot"))
    }

    // The whole path over a socket: asked once the herd names a machine that keeps sessions, shown
    // from the node's answer, and the pane a resume made is opened when it arrives.
    @Test
    fun theAppAsksOnConnectAndOpensWhatAResumeMade() = asksAndOpens(paneFirst = false)

    // A session op settles the herd before it acks (`spawn_settle`), so the pane can be in hand
    // before the ack that names it, and no later frame is owed to open it.
    @Test
    fun aPaneThatArrivedBeforeTheResumeAckIsStillOpened() = asksAndOpens(paneFirst = true)

    private fun asksAndOpens(paneFirst: Boolean) {
        val node = StubNode(MANAGING_HELLO).apply { start() }
        val scope = CoroutineScope(SupervisorJob())
        val app = AppState(scope, KamprStore(), MemoryPrefs(), null, NoPush()).apply { go(Screen.Herd) }
        val manage = AppManage(app)
        try {
            runBlocking {
                app.connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
                until("the session") { app.store.status.value is ConnectionStatus.Live }
                node.push(HERD_JSON)
                val ask = Regex(""""rid":"([^"]+)"""")
                until("the ask") { node.received.any { "native.resumable" in it && HUB in it } }
                val rid = node.received.first { "native.resumable" in it && HUB in it }.let { ask.find(it)!!.groupValues[1] }
                node.push(
                    """{"t":"managed","op":"native.resumable","ok":true,"rid":"$rid","resumable":[""" +
                        """{"id":"01k8/74877","label":"work","argv":["/usr/bin/nvim","-S","/home/u/s.vim"],"cwd":"/home/u/dev"}]}""",
                )
                until("the offer") { manage.resumable(HUB) == listOf(NVIM) }

                manage.resume(HUB, NVIM)
                until("the resume") { node.received.any { "\"native.resume\"" in it } }
                val ran = node.received.first { "\"native.resume\"" in it }
                val acked = """{"t":"managed","op":"native.resume","ok":true,"rid":"${ask.find(ran)!!.groupValues[1]}","id":"$HUB/n:s9"}"""
                val made = """{"t":"herd","nodes":[{"id":"$HUB","name":"hub","kind":"local","online":true,"native":true,"resume":true}],""" +
                    """"panes":[{"id":"$HUB/n:s9","node_id":"$HUB","provider":"native","updated_at":"7"}]}"""
                if (paneFirst) {
                    node.push(made)
                    until("the herd") { app.store.herd.value.panes.any { it.id == "$HUB/n:s9" } }
                    node.push(acked)
                } else {
                    node.push(acked)
                    node.push(made)
                }
                until("the pane") { (app.screen as? Screen.Pane)?.paneId == "$HUB/n:s9" }
            }
        } finally {
            app.connection.disconnect()
            scope.cancel()
            node.stop()
        }
    }
}

private const val MANAGING_HELLO =
    """{"t":"hello","protocol":1,"node_id":"$HUB","node_name":"hub","build":"test",""" +
        """"role":"full","caps":{"manage":true},"security":{"tier":0,"passkeys":false}}"""
