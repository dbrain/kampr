package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.Abilities
import dev.kampr.shared.wire.Billing
import dev.kampr.shared.wire.Limits
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.SessionFacts
import kotlinx.serialization.json.JsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals

private const val PANE = "01JNODE/c:s1"

private object UsageSurfaces : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) = Box(modifier)
}

private fun seam(facts: List<String>): PaneState = KamprStore().run {
    accept(ServerMsg.ConvoAbilities(PANE, Abilities(facts = facts)))
    accept(ServerMsg.ConvoSession(PANE, SessionFacts("s1", billing = Billing(provider = "Anthropic"))))
    accept(ServerMsg.ConvoLimits(PANE, Limits("Max 5x", JsonPrimitive(72), "2026-11-27T14:30:00Z")))
    pane(PANE)
}

private val INFO = PaneInfo(
    id = PANE,
    nodeId = "01JNODE",
    workspace = "kampr",
    tab = "1",
    cwd = "/tmp",
    agent = "claude",
    agentStatus = "idle",
    cols = 80,
    rows = 24,
    hasConversation = true,
    converses = true,
)

@OptIn(ExperimentalTestApi::class)
class UsageChipTest {
    // Usage, its reset and who pays were only in the facts line under the reply box.
    @Test
    fun theHeaderSaysUsageWhereTheSourceDeclaresIt() {
        for ((facts, chips) in listOf(listOf("limits", "billing") to 1, listOf("context") to 0)) {
            runComposeUiTest {
                setContent {
                    CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                        Box(Modifier.size(411.dp, 914.dp)) {
                            PaneScreenMobile(seam(facts), INFO, PaneView.Conversation, UsageSurfaces, landscape = false, readOnly = false, onBack = {}, onView = {}, modifier = Modifier.fillMaxSize())
                        }
                    }
                }
                waitForIdle()
                val found = onAllNodesWithContentDescription("Usage: Max 5x · 72% · resets", substring = true, useUnmergedTree = true).fetchSemanticsNodes()
                assertEquals(chips, found.size.coerceAtMost(1), "$facts")
            }
        }
    }
}
