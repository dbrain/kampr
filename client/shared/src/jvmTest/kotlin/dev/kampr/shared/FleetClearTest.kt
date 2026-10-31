package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.FleetScreen
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.wire.FleetInfo
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals

private fun run(id: String, cohort: String, state: String, started: Long, exit: Int? = 0) = PaneInfo(
    id = id,
    nodeId = "n1",
    fleet = FleetInfo(cohort = cohort, command = "uptime $cohort", state = state, exitCode = exit.takeIf { state == "exited" }, startedUnix = started),
)

private val BOARD = Herd(
    nodes = listOf(NodeInfo(id = "n1", name = "n1", kind = "peer", online = true)),
    panes = listOf(
        run("n1/fleet:a1", "done", "exited", 1_700_000_000),
        run("n1/fleet:a2", "done", "exited", 1_700_000_000, exit = 1),
        run("n1/fleet:b1", "half", "running", 1_700_000_100),
        run("n1/fleet:b2", "half", "exited", 1_700_000_100),
    ),
    known = true,
)

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

// The node keeps every finished run until it is forgotten, so a board with no way to forget one
// only ever grows.
@OptIn(ExperimentalTestApi::class)
class FleetClearTest {
    @Test
    fun clearingTheBoardForgetsEveryFinishedRunAndNoLiveOne() = runComposeUiTest {
        val forgotten = render()
        onNodeWithContentDescription("Clear every finished run").performClick()
        assertEquals(setOf("n1/fleet:a1", "n1/fleet:a2", "n1/fleet:b2"), forgotten.toSet())
    }

    @Test
    fun aFinishedFanOutClearsOnItsOwnAndOneStillRunningOffersNothing() = runComposeUiTest {
        val forgotten = render()
        onAllNodesWithContentDescription("Clear uptime", substring = true).assertCountEquals(1)
        onNodeWithContentDescription("Clear uptime done").performClick()
        assertEquals(setOf("n1/fleet:a1", "n1/fleet:a2"), forgotten.toSet())
    }

    private fun ComposeUiTest.render(): MutableList<String> {
        val forgotten = mutableListOf<String>()
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokens(),
                LocalConnectionStatus provides ConnectionStatus.Live("full"),
            ) {
                Box(Modifier.size(411.dp, 891.dp)) {
                    FleetScreen(
                        herd = BOARD,
                        breakpoint = Breakpoint.Portrait,
                        onOpenPane = {},
                        onAnswer = { _, _ -> },
                        onStop = {},
                        onRun = {},
                        canRun = true,
                        onForget = { forgotten += it },
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        waitForIdle()
        return forgotten
    }
}
