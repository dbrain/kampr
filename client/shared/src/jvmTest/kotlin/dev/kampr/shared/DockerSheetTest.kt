package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.v2.runComposeUiTest
import dev.kampr.shared.model.ContainerWatch
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppManage
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.DockerSheet
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.Sheet
import dev.kampr.shared.wire.ContainerInfo
import dev.kampr.shared.wire.DockerAction
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import dev.kampr.shared.wire.dockerDetailOf
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.serialization.json.JsonObject
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val NODE_ID = "01JHOST"
private const val CID = "f49c60fac115"
private const val NOW_SEC = 1_794_220_000L

private const val MANAGING =
    """{"t":"hello","protocol":1,"node_id":"01JHOST","node_name":"front","build":"test",""" +
        """"role":"full","caps":{"manage":true},"security":{"tier":0,"passkeys":false}}"""

private const val HERD =
    """{"t":"herd","nodes":[{"id":"01JHOST","name":"front","kind":"local","online":true,"docker":true,""" +
        """"containers":[{"id":"$CID","name":"kobbler-postgres-test-1","image":"postgres:18.6","state":"running"}]}],"panes":[]}"""

private val DETAIL = """
{"id":"$CID","name":"kobbler-postgres-test-1","image":"postgres:18.6","state":"running","health":"unhealthy",
 "started_at":${NOW_SEC - 600},"restarts":2,
 "env":["POSTGRES_DB=kobbler"],"mounts":[{"src":"/srv/pg","dst":"/var/lib/postgresql","mode":"rw"}],
 "ports":["5432→5432/tcp"],
 "health_log":[{"exit_code":1,"output":"pg_isready: no response"}],
 "logs":[{"stream":"stderr","line":"FATAL: the database system is starting up"}],
 "stats":{"cpu_pct":12.5,"mem_used":44040192,"mem_limit":1073741824},
 "events":[{"at":${NOW_SEC - 300},"action":"oom"},{"at":${NOW_SEC - 300},"action":"die","exit_code":137}]}
"""

private const val REFUSED = """{"t":"managed","op":"docker.restart","ok":false,"code":"docker","message":"no such container"}"""

private const val SHELL_MADE = """{"t":"managed","op":"docker.shell","ok":true,"id":"01JHOST/n:sh1"}"""

private const val SHELL_PANE = """{"id":"01JHOST/n:sh1","node_id":"01JHOST","provider":"native","container":"kobbler-postgres-test-1","updated_at":"7"}"""

private fun detailAck(detail: String = DETAIL) = """{"t":"managed","op":"docker.detail","ok":true,"detail":$detail}"""

private fun op(action: DockerAction) = ManageOp.Docker(action, NODE_ID, CID)

private val NODE = NodeInfo(id = NODE_ID, name = "front", kind = "local")

private val RUNNING = ContainerInfo(id = CID, name = "kobbler-postgres-test-1", image = "postgres:18.6", state = "running")

@OptIn(ExperimentalTestApi::class, ExperimentalCoroutinesApi::class)
class DockerSheetTest {
    @Test
    fun anOpenSheetAsksForTheDetailAtOnceAndEveryThreeSecondsUntilItCloses() {
        val scope = TestScope()
        val sent = mutableListOf<ManageOp>()
        val watch = ContainerWatch(scope.backgroundScope, send = { sent += it })
        watch.watch(NODE_ID, CID)
        scope.runCurrent()
        assertEquals(listOf<ManageOp>(op(DockerAction.Detail)), sent)
        scope.advanceTimeBy(3_001)
        assertEquals(2, sent.size)
        scope.advanceTimeBy(3_000)
        assertEquals(3, sent.size)
        watch.stop()
        scope.advanceTimeBy(10_000)
        assertEquals(3, sent.size, "a closed sheet kept polling")
    }

    @Test
    fun aDetailForAnotherContainerIsNotThisOnes() {
        val scope = TestScope()
        val watch = ContainerWatch(scope.backgroundScope, send = {})
        watch.watch(NODE_ID, CID)
        watch.acked(Wire.decode(detailAck("""{"id":"other","name":"x","state":"running"}""")) as ServerMsg.Managed)
        assertNull(watch.detail)
        watch.acked(Wire.decode(detailAck()) as ServerMsg.Managed)
        assertEquals(12.5, watch.detail?.more?.stats?.cpuPct)
        watch.stop()
        assertNull(watch.detail)
    }

    @Test
    fun theAppOpensTheSheetHoldsTheDetailAndStopsWatchingOnClose() {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        try {
            val store = KamprStore()
            val state = AppState(scope, store, MemoryPrefs(), null).apply { go(Screen.Herd) }
            store.accept(Wire.decode(MANAGING)!!)
            store.accept(Wire.decode(HERD)!!)
            AppManage(state).openContainer(NODE_ID, CID)
            assertEquals(Sheet.Container(NODE_ID, CID), state.sheet)
            assertEquals(NODE_ID to CID, state.containers.watching)
            store.accept(Wire.decode(detailAck())!!)
            assertEquals("pg_isready: no response", state.containers.detail?.more?.healthLog?.single()?.output)
            state.closeSheet()
            assertNull(state.containers.watching)
            AppManage(state).openContainer(NODE_ID, CID)
            state.go(Screen.Fleet)
            assertNull(state.containers.watching, "leaving the screen closes the sheet and its poll")
        } finally {
            scope.cancel()
        }
    }

    private fun sheet(
        container: ContainerInfo,
        sent: MutableList<ManageOp>,
        refusal: () -> String? = { null },
    ): @androidx.compose.runtime.Composable () -> Unit = {
        CompositionLocalProvider(LocalTokens provides phoneTokens()) {
            Box(Modifier.fillMaxSize()) {
                DockerSheet(
                    breakpoint = Breakpoint.Portrait,
                    node = NODE,
                    container = container,
                    detail = dockerDetailOf(Wire.json.parseToJsonElement(DETAIL) as JsonObject),
                    refusal = refusal(),
                    now = NOW_SEC * 1000.0,
                    onManage = { sent += it },
                    onDismiss = {},
                )
            }
        }
    }

    @Test
    fun theSheetSaysWhyAndWhatItIsUsing() = runComposeUiTest {
        setContent(sheet(RUNNING, mutableListOf()))
        for (text in listOf(
            "pg_isready: no response",
            "FATAL: the database system is starting up",
            "POSTGRES_DB=kobbler",
            "/srv/pg → /var/lib/postgresql",
            "5432→5432/tcp",
            "12.5%",
            "42 MB of 1.0 GB",
        )) {
            assertTrue(onAllNodesWithText(text, substring = true).fetchSemanticsNodes().isNotEmpty(), "missing: $text")
        }
        assertTrue(onAllNodesWithText("die 137", substring = true).fetchSemanticsNodes().isNotEmpty(), "crash timeline")
    }

    @Test
    fun aFailingAdoptionSaysWhyAtTheTop() = runComposeUiTest {
        val why = "kobbler-postgres-test-1 has a read-only root filesystem, so kampr can't be copied into it"
        setContent(sheet(RUNNING.copy(child = "01JHOST-c-$CID", adoptError = why), mutableListOf()))
        assertEquals(1, onAllNodesWithText(why).fetchSemanticsNodes().size)
    }

    @Test
    fun eachActionSendsItsOpWithNoConfirmation() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent(sheet(RUNNING, sent))
        for ((label, action) in listOf(
            "Restart" to DockerAction.Restart,
            "Stop" to DockerAction.Stop,
            "Adopt" to DockerAction.Adopt,
            "Shell in" to DockerAction.Shell,
        )) {
            onNodeWithContentDescription(label).performClick()
            assertEquals(op(action), sent.last(), label)
        }
        assertEquals(0, onAllNodesWithContentDescription("Start").fetchSemanticsNodes().size)
        assertEquals(0, onAllNodesWithContentDescription("Release").fetchSemanticsNodes().size)
    }

    @Test
    fun aStoppedAdoptedContainerOffersStartAndRelease() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent(sheet(RUNNING.copy(state = "exited", exitCode = 1, child = "01JHOST-c-$CID"), sent))
        onNodeWithContentDescription("Start").performClick()
        onNodeWithContentDescription("Release").performClick()
        assertEquals(listOf<ManageOp>(op(DockerAction.Start), op(DockerAction.Release)), sent)
        assertEquals(0, onAllNodesWithContentDescription("Stop").fetchSemanticsNodes().size)
        assertEquals(0, onAllNodesWithContentDescription("Shell in").fetchSemanticsNodes().size)
    }

    @Test
    fun aRefusalIsSaidInTheSheet() = runComposeUiTest {
        var refusal by mutableStateOf<String?>(null)
        setContent(sheet(RUNNING, mutableListOf()) { refusal })
        refusal = "no such container"
        waitForIdle()
        assertNotNull(onAllNodesWithText("no such container").fetchSemanticsNodes().firstOrNull())
    }

    // The shell opens the way every other made pane does, when the patch carrying it lands.
    @Test
    fun aShellOpensThePaneTheAckNamesAndARefusalReachesTheSheet() {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        try {
            val store = KamprStore()
            val state = AppState(scope, store, MemoryPrefs(), null).apply { go(Screen.Herd) }
            store.accept(Wire.decode(MANAGING)!!)
            store.accept(Wire.decode(HERD)!!)
            state.openContainer(NODE_ID, CID)
            store.accept(Wire.decode(REFUSED)!!)
            assertEquals("no such container", state.containers.refusal)
            state.containers.act(op(DockerAction.Restart))
            assertNull(state.containers.refusal, "a new press clears the old refusal")
            store.accept(Wire.decode(SHELL_MADE)!!)
            store.accept(Wire.decode(detailAck())!!)
            assertNull(state.sheet, "the sheet gives way to the shell")
            store.accept(Wire.decode(HERD.replace("\"panes\":[]", "\"panes\":[$SHELL_PANE]"))!!)
            assertEquals("$NODE_ID/n:sh1", (state.screen as Screen.Pane).paneId)
        } finally {
            scope.cancel()
        }
    }
}
