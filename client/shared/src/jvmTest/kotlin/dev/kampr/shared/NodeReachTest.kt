package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
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
import dev.kampr.shared.ui.HerdPortrait
import dev.kampr.shared.ui.nodeReach
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), content = content)
}

private val LOCAL = NodeInfo(id = "01JA", name = "comingclean", kind = "local")
private val PEER = NodeInfo(id = "01JB", name = "pleader", kind = "peer", rttMs = 4.0)
private val HERD = Herd(
    nodes = listOf(LOCAL, PEER),
    panes = listOf(
        PaneInfo(id = "01JA/w1:p1", nodeId = "01JA", label = "shell"),
        PaneInfo(id = "01JB/w1:p1", nodeId = "01JB", label = "shell"),
    ),
    known = true,
)

// A node reached through the one this client is connected to is a *peer*, and that is the whole of
// what the wire says about it (`kind`). Three surfaces called it a "tailnet" instead — a claim
// about the transport nothing measured, contradicted by the setup screen's own promise that Kampr
// "never assumes a tailnet", and wrong on the operator's herd, which is three machines on a LAN.
@OptIn(ExperimentalTestApi::class)
class NodeReachTest {
    @Test
    fun theWordForANodeIsWhatTheWireSaysAndNotAGuessAtItsNetwork() {
        assertEquals("local", nodeReach(LOCAL))
        assertEquals("peer", nodeReach(PEER))
    }

    @Test
    fun noSurfaceClaimsAHerdIsOnATailnet() = runComposeUiTest {
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    HerdPortrait(HERD, ConnectionStatus.Live("full"), 0.0, 12.0, emptyList(), {}, null)
                }
            }
        }
        waitForIdle()
        assertTrue(
            onAllNodesWithText("tailnet", substring = true, useUnmergedTree = true).fetchSemanticsNodes().isEmpty(),
            "the herd screen told the operator their LAN was a tailnet",
        )
        assertTrue(
            onAllNodesWithContentDescription("tailnet", substring = true, useUnmergedTree = true)
                .fetchSemanticsNodes().isEmpty(),
            "a screen reader was told the same thing",
        )
        assertTrue(
            onAllNodesWithText("peer · ", substring = true, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty(),
            "the peer's header lost the one word the wire does support",
        )
    }
}
