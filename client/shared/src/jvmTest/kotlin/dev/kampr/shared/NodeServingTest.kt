package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.serves
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.NodeQuiet
import dev.kampr.shared.ui.nodeFacts
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals

private const val NO_HERDR = "herdr socket /run/user/1000/herdr/default.sock: No such file or directory"

private fun node(json: String): NodeInfo {
    val msg = Wire.decode("""{"t":"herd","nodes":[$json],"panes":[]}""") as ServerMsg.Herd
    return msg.nodes.single()
}

private val NATIVE = node("""{"id":"01JN","name":"shed","kind":"peer","online":false,"reachable":true,"native":true,"detail":"$NO_HERDR"}""")
private val COLD = node("""{"id":"01JC","name":"coldbox","kind":"peer","online":false,"reachable":true,"detail":"$NO_HERDR"}""")
private val GONE = node("""{"id":"01JG","name":"gonebox","kind":"peer","online":false,"reachable":false,"native":true,"detail":"unreachable"}""")

private val NATIVE_PANE = PaneInfo("01JN/n:01ja", "01JN", label = "build", provider = "native", sessionId = "01ja")
private val HERDR_PANE = PaneInfo("01JN/w1:p1", "01JN", label = "shell")

// A machine with no herdr and native sessions was drawn as a machine that is gone: ring, "offline",
// "herdr is not running". Its sessions worked. herdr's absence is still true, so it is said — once,
// quietly — and a herdr-only machine losing herdr must read exactly as it did.
@OptIn(ExperimentalTestApi::class)
class NodeServingTest {
    @Test
    fun aMachineIsServingWhenItAnswersAndHasHerdrOrNativeSessions() {
        assertEquals(true, NATIVE.isServing)
        assertEquals(false, COLD.isServing)
        assertEquals(false, GONE.isServing)
        assertEquals(true, NodeInfo("01JL", kind = "local").isServing)
    }

    @Test
    fun aNativeMachineReadsOnlineAndSaysItHasNoHerdr() {
        assertEquals(
            listOf("peer", "online", "no herdr", NO_HERDR),
            nodeFacts(NATIVE, withStatus = true),
        )
        assertEquals(listOf("peer", "offline", NO_HERDR), nodeFacts(COLD, withStatus = true))
        assertEquals(listOf("peer", "offline", "unreachable"), nodeFacts(GONE, withStatus = true))
    }

    @Test
    fun aNativePaneNeedsOnlyTheNodeAndAHerdrPaneNeedsItsHerdr() {
        assertEquals(true, NATIVE.serves(NATIVE_PANE))
        assertEquals(false, NATIVE.serves(HERDR_PANE))
        assertEquals(false, GONE.serves(NATIVE_PANE))
        assertEquals(false, COLD.serves(HERDR_PANE))
    }

    @Test
    fun anEmptyNativeMachineIsNotToldItsHerdrIsTheThingToStart() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(411.dp, 914.dp)) { NodeQuiet(NATIVE) }
            }
        }
        waitForIdle()
        onNodeWithText("nothing running on this machine · no herdr").assertIsDisplayed()
    }

    @Test
    fun theMachinePillCountsANativeMachine() = runComposeUiTest {
        val herd = Herd(nodes = listOf(NATIVE, COLD), panes = listOf(NATIVE_PANE), known = true)
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(420.dp, 900.dp)) {
                    HerdPortrait(herd, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null)
                }
            }
        }
        waitForIdle()
        onNodeWithContentDescription("Machines — 1 node online").assertExists()
    }
}
