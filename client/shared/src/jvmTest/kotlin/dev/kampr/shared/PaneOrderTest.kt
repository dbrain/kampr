package dev.kampr.shared

import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.groups
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals

// The sibling of NamingParityTest, for the same reason: `sidebar.rs` and `Herd.kt` rank the same
// five states, and two clients that order one herd differently are two different products.
class PaneOrderTest {
    private fun pane(id: String, status: String) =
        PaneInfo(id = id, nodeId = "n", agent = "claude", agentStatus = status)

    // Herdr rolls a tab up as blocked > done > working > idle > unknown, measured by driving two
    // panes through every pair and reading `tab.get`. `done` is only ever synthesised for a pane
    // that finished while unfocused, so it is an unread marker: news, where `working` is not.
    @Test
    fun aPaneThatFinishedUnwatchedSitsAboveOneThatIsStillWorking() {
        val herd = Herd(
            nodes = listOf(NodeInfo(id = "n", kind = "local")),
            panes = listOf(
                pane("n/w1:p1", "working"),
                pane("n/w1:p2", "done"),
                pane("n/w1:p3", "blocked"),
                pane("n/w1:p4", "idle"),
                pane("n/w1:p5", "unknown"),
            ),
        )

        val order = herd.groups().single().panes.map { it.agentStatus }

        assertEquals(listOf("blocked", "done", "working", "idle", "unknown"), order)
    }
}
