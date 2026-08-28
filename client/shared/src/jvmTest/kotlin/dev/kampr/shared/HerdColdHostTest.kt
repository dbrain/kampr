package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.wire.NodeInfo
import kotlin.test.Test

// The screen a rarely-visited host opens on: the node answers, its herdr does not, and there are
// no panes to draw. It used to say "This node is up and has nothing running on it", which is true
// of the node and false about the machine — the operator is looking at a herd with nothing in it
// and no word anywhere about why, or that the + in the bar is what fixes it.
private val COLD = NodeInfo(id = "01JCOLD", name = "coldbox", kind = "local", online = false)
private val LIVE = NodeInfo(id = "01JLIVE", name = "comingclean", kind = "local", online = true)

@OptIn(ExperimentalTestApi::class)
class HerdColdHostTest {
    private fun herdOf(vararg nodes: NodeInfo) = Herd(nodes = nodes.toList(), panes = emptyList())

    @Test
    fun anEmptyHerdWhoseHerdrIsStoppedSaysSoAndSaysWhatStartsIt() = runComposeUiTest {
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    HerdPortrait(herdOf(COLD), ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null)
                }
            }
        }
        onNodeWithContentDescription("herdr is not running on coldbox", substring = true).assertExists()
        onNodeWithContentDescription("New", substring = true).assertExists()
    }

    @Test
    fun anEmptyHerdWhoseHerdrIsRunningStillSaysThereIsNothingOnIt() = runComposeUiTest {
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    HerdPortrait(herdOf(LIVE), ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null)
                }
            }
        }
        onNodeWithContentDescription("nothing running on this machine", substring = true).assertExists()
    }
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), content = content)
}
