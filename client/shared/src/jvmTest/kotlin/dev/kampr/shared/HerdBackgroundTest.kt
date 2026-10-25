package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.groups
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals

// docs/18: a muffit session no terminal is showing is listed apart from the machine's terminals,
// a TUI's terminal among them.
private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local", online = true)

private fun pane(id: String, provider: String?, agent: String?, status: String) = PaneInfo(
    id = "01JNODE/$id",
    nodeId = NODE.id,
    agent = agent,
    agentStatus = status,
    provider = provider,
    title = id,
)

private val HERD = Herd(
    nodes = listOf(NODE),
    panes = listOf(
        pane("m:wake", "muffit", "muffit", "blocked"),
        pane("n:tui", "native", "muffit", "working"),
        pane("w1:p1", null, "claude", "idle"),
    ),
)

@OptIn(ExperimentalTestApi::class)
class HerdBackgroundTest {
    @Test
    fun backgroundSessionsComeAfterTheTerminalsUnderTheirOwnHeading() = runComposeUiTest {
        val group = HERD.groups().single()
        assertEquals(listOf("01JNODE/n:tui", "01JNODE/w1:p1"), group.terminals.map { it.id })
        assertEquals(listOf("01JNODE/m:wake"), group.background.map { it.id })

        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(420.dp, 1200.dp)) {
                    HerdPortrait(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null)
                }
            }
        }
        onNodeWithContentDescription("Background · muffit").assertExists()
    }
}
