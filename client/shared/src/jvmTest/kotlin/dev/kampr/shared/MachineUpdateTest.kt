package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.SetupScreen
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.Security
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

private val CURRENT = NodeInfo(id = "01JFRONT", name = "front", kind = "local", build = "0.1.2")
private val STALE = NodeInfo(
    id = "01JBACK",
    name = "back",
    kind = "peer",
    build = "0.1.0",
    update = "0.1.2",
)

// "Which of my machines are stale" is the question the Machines list exists to answer once every
// node names its own build. These check that it answers it, and that it stays quiet when the
// answer is "none of them".
@OptIn(ExperimentalTestApi::class)
class MachineUpdateTest {
    @Composable
    private fun Setup(nodes: List<NodeInfo>) {
        Box(Modifier.size(420.dp, 1400.dp)) {
            SetupScreen(
                status = null,
                security = Security(),
                running = true,
                endpoint = null,
                nodes = nodes,
                pairingCode = null,
                pairingError = null,
                onConnect = {},
                onPairingCode = {},
                onOpenHerd = {},
                onDevices = {},
                onAppearance = {},
                onNotifications = {},
            )
        }
    }

    @Test
    fun aStaleMachineSaysWhichReleaseWouldFixIt() = runComposeUiTest {
        setContent { Themed { Setup(listOf(CURRENT, STALE)) } }
        waitForIdle()
        assertTrue(
            onAllNodesWithText("0.1.2 available", substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the herd had a node a release behind and the list did not say so",
        )
        assertEquals(
            1,
            onAllNodesWithText("available", substring = true).fetchSemanticsNodes().size,
            "a machine that is current claimed an update was waiting",
        )
    }

    // The whole list is quiet when every machine is current: an update line that is always there
    // is a line nobody reads.
    @Test
    fun aHerdThatIsAllCurrentSaysNothingAtAll() = runComposeUiTest {
        setContent { Themed { Setup(listOf(CURRENT, CURRENT.copy(id = "01JB", name = "back"))) } }
        waitForIdle()
        assertTrue(
            onAllNodesWithText("available", substring = true).fetchSemanticsNodes().isEmpty(),
            "a herd with nothing to do still had an update line on it",
        )
    }

    // The row is spoken as one sentence, the way every other status row in Kampr is — not as a
    // second vocabulary bolted on beside the name and the build.
    @Test
    fun theRowIsSpokenAsOneSentenceThatIncludesTheUpdate() = runComposeUiTest {
        setContent { Themed { Setup(listOf(STALE)) } }
        waitForIdle()
        onNodeWithContentDescription("back, peer, online, kampr 0.1.0, 0.1.2 available").assertExists()
    }

    @Test
    fun aCurrentMachineIsSpokenWithoutAnUpdateClause() = runComposeUiTest {
        setContent { Themed { Setup(listOf(CURRENT)) } }
        waitForIdle()
        onNodeWithContentDescription("front, this machine, online, kampr 0.1.2").assertExists()
    }
}
