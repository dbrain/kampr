package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.ConnectPanel
import dev.kampr.shared.ui.SetupScreen
import dev.kampr.shared.wire.Security
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), content = content)
}

// The first run on a phone reaches this panel and nothing else, so every affordance it needs has
// to be here — reachable by name, not merely painted.
@OptIn(ExperimentalTestApi::class)
class ConnectPanelTest {
    @Test
    fun aBareHostAndPortIsAllThatHasToBeTyped() = runComposeUiTest {
        var dialled: Endpoint? = null
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    ConnectPanel(null, null, { dialled = it })
                }
            }
        }
        onNodeWithContentDescription("Connect to this node").assertIsNotEnabled()
        onNodeWithContentDescription("Node address").performTextInput("192.168.1.24:8790")
        onNodeWithContentDescription("Connect to this node").assertIsEnabled().performClick()
        waitForIdle()
        assertEquals(Endpoint("http://192.168.1.24:8790"), dialled)
    }

    @Test
    fun anAddressUsedBeforeIsOneTap() = runComposeUiTest {
        var dialled: Endpoint? = null
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    ConnectPanel(null, null, { dialled = it }, recent = listOf("https://kampr.example.com"))
                }
            }
        }
        onNodeWithContentDescription("Use https://kampr.example.com").performClick()
        onNodeWithContentDescription("Connect to this node").performClick()
        waitForIdle()
        assertEquals(Endpoint("https://kampr.example.com"), dialled)
    }

    // A code scanned off the desktop's QR arrives armed but not spent: the tap is still the
    // operator's, and the enrolment is still one they can see happening.
    @Test
    fun aScannedCodeArrivesInTheFieldAndIsRedeemedOnTheTap() = runComposeUiTest {
        var dialled: Endpoint? = null
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    ConnectPanel(
                        Endpoint("http://192.168.1.24:8790"),
                        null,
                        { dialled = it },
                        offeredCode = "K7QF2M",
                    )
                }
            }
        }
        assertNull(dialled, "a scan must not enrol on its own")
        onNodeWithContentDescription("Connect to this node").performClick()
        waitForIdle()
        assertEquals(Endpoint("http://192.168.1.24:8790", "K7QF2M"), dialled)
    }

    // The whole point of the change: with nothing stored and nothing derivable, the screen the
    // app opens on is the one that asks — not a herd behind a failure.
    @Test
    fun theSetupScreenAsksForAnAddressWhenThereIsNone() = runComposeUiTest {
        setContent {
            Themed {
                Box(Modifier.size(420.dp, 900.dp)) {
                    SetupScreen(
                        status = null,
                        security = Security(),
                        running = false,
                        endpoint = null,
                        nodes = emptyList(),
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
        }
        onNodeWithContentDescription("Node address").assertExists()
        onNodeWithContentDescription("Connect to this node").assertExists()
    }
}
