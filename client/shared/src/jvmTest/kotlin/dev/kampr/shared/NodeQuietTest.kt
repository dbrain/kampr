package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.NodeQuiet
import dev.kampr.shared.wire.NodeInfo
import kotlin.test.Test

// The line under a machine with nothing running on it, and the surface the operator actually met
// the cold-host bug on: the sidebar showed a rebooted peer with its herdr socket error under it and
// no word that the + would start one. `NewSheetColdHostTest` is the same distinction on the sheet
// that acts; this is the one that explains.
private const val SOCKET_ERROR = "herdr socket /run/user/1000/herdr/default.sock: No such file or directory"

private val COLD_LOCAL = NodeInfo("01JCOLD", "coldbox", "local", online = false, detail = SOCKET_ERROR)
private val COLD_PEER =
    NodeInfo("01JSHED", "shed", "peer", online = false, detail = SOCKET_ERROR, reachable = true)
private val GONE_PEER = NodeInfo("01JGONE", "gonebox", "peer", online = false, detail = "unreachable")
private val LIVE = NodeInfo("01JHUB", "comingclean", "local")

@OptIn(ExperimentalTestApi::class)
class NodeQuietTest {
    private fun ComposeUiTest.quiet(node: NodeInfo) {
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(411.dp, 914.dp)) { NodeQuiet(node) }
            }
        }
        waitForIdle()
    }

    @Test
    fun aLocalMachineWhoseHerdrIsStoppedIsToldHowToStartIt() = runComposeUiTest {
        quiet(COLD_LOCAL)
        onNodeWithText("herdr is not running on coldbox — New (+) starts it").assertIsDisplayed()
    }

    @Test
    fun aPeerWhoseHerdrIsStoppedIsToldTheSameThing() = runComposeUiTest {
        quiet(COLD_PEER)
        onNodeWithText("herdr is not running on shed — New (+) starts it").assertIsDisplayed()
    }

    @Test
    fun aMachineWithNoNodeBehindItStillSaysWhyItIsGone() = runComposeUiTest {
        quiet(GONE_PEER)
        onNodeWithText("unreachable").assertIsDisplayed()
    }

    @Test
    fun aLiveMachineWithNothingOnItSaysThat() = runComposeUiTest {
        quiet(LIVE)
        onNodeWithText("nothing running on this machine").assertIsDisplayed()
    }
}
