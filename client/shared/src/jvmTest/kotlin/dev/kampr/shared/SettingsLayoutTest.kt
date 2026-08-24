package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Dp
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
import kotlin.test.assertTrue

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), content = content)
}

private val NODE = NodeInfo(id = "01JFRONT", name = "front", kind = "local", build = "0.1.2")
private const val NODE_ROW = "front, this machine, online, kampr 0.1.2"
private const val LAST_RUNG = "Add another machine"

@Composable
private fun Setup(width: Dp, height: Dp, wide: Boolean) {
    Box(Modifier.size(width, height)) {
        SetupScreen(
            status = null,
            security = Security(),
            running = true,
            endpoint = null,
            nodes = listOf(NODE),
            pairingCode = null,
            pairingError = null,
            onConnect = {},
            onPairingCode = {},
            onDevices = {},
            onAppearance = {},
            onNotifications = {},
            wide = wide,
        )
    }
}

// Settings was a phone-width column whatever the window, and it carried a second door to the herd
// that the tab bar and the sidebar already are.
@OptIn(ExperimentalTestApi::class)
class SettingsLayoutTest {
    @Test
    fun settingsDoesNotOfferASecondWayToOpenTheHerd() = runComposeUiTest {
        setContent { Themed { Setup(420.dp, 1400.dp, wide = false) } }
        waitForIdle()
        assertTrue(
            onAllNodesWithText("Open the herd", substring = true, useUnmergedTree = true).fetchSemanticsNodes().isEmpty(),
            "settings still paints a button for what the navigation already does",
        )
        assertTrue(
            onAllNodesWithContentDescription("Open the herd", substring = true).fetchSemanticsNodes().isEmpty(),
            "settings still speaks a button for what the navigation already does",
        )
    }

    // Two columns is the claim, and the only honest evidence for it is that the machines list is
    // beside the ladder rather than under it.
    @Test
    fun aDesktopWindowPutsTheSettingsCardsBesideEachOtherRatherThanUnderneath() = runComposeUiTest {
        setContent { Themed { Setup(1200.dp, 1000.dp, wide = true) } }
        waitForIdle()
        val machines = onNodeWithContentDescription(NODE_ROW).getUnclippedBoundsInRoot()
        val ladder = onNodeWithText(LAST_RUNG).getUnclippedBoundsInRoot()
        assertTrue(
            machines.left > ladder.right,
            "the machines list was not in a column of its own: it starts at ${machines.left}, " +
                "and the ladder still runs to ${ladder.right}",
        )
        assertTrue(
            machines.top < ladder.top,
            "the second column starts below the first, so this is one column with a gap in it",
        )
    }

    @Test
    fun aPhoneKeepsTheSingleColumn() = runComposeUiTest {
        setContent { Themed { Setup(420.dp, 1400.dp, wide = false) } }
        waitForIdle()
        val machines = onNodeWithContentDescription(NODE_ROW).getUnclippedBoundsInRoot()
        val ladder = onNodeWithText(LAST_RUNG).getUnclippedBoundsInRoot()
        assertTrue(machines.left < ladder.right, "a phone-width window split settings into columns")
        assertTrue(machines.top > ladder.top, "the machines list did not follow the ladder down the page")
    }

    // The desktop breakpoint is not a promise of width: this screen shares its window with the
    // sidebar, and a narrow detail column has no room for two measures of readable text.
    @Test
    fun aDesktopWindowTooNarrowForTwoMeasuresKeepsOne() = runComposeUiTest {
        setContent { Themed { Setup(700.dp, 1400.dp, wide = true) } }
        waitForIdle()
        val machines = onNodeWithContentDescription(NODE_ROW).getUnclippedBoundsInRoot()
        val ladder = onNodeWithText(LAST_RUNG).getUnclippedBoundsInRoot()
        assertTrue(machines.left < ladder.right, "700 dp was split into two columns of unreadable measure")
        assertTrue(machines.top > ladder.top, "the machines list did not follow the ladder down the page")
    }
}
