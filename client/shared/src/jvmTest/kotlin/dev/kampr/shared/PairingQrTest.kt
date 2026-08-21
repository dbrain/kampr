package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.SetupStatus
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.SetupScreen
import dev.kampr.shared.wire.Security
import kotlin.test.Test

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), content = content)
}

// The wizard's QR is for the *other* device: Kampr is open on a desk and the phone being enrolled
// is the thing holding the camera. A phone-width portrait layout is that other device, so a
// picture of the address it is already at is clutter.
@OptIn(ExperimentalTestApi::class)
class PairingQrTest {
    private fun setup(wide: Boolean, code: String?): @Composable () -> Unit = {
        Themed {
            Box(Modifier.size(if (wide) 1200.dp else 390.dp, 900.dp)) {
                SetupScreen(
                    status = SetupStatus("http://192.168.1.24:8790", devices = 1),
                    security = Security(),
                    running = true,
                    endpoint = Endpoint("http://192.168.1.24:8790", "tok"),
                    nodes = emptyList(),
                    pairingCode = code,
                    pairingError = null,
                    onConnect = {},
                    onPairingCode = {},
                    onOpenHerd = {},
                    onDevices = {},
                    onAppearance = {},
                    onNotifications = {},
                    wide = wide,
                )
            }
        }
    }

    @Test
    fun aDesktopWizardShowsTheCodeAPhoneCanScan() = runComposeUiTest {
        setContent(setup(wide = true, code = "K7QF2M"))
        onNodeWithContentDescription(
            "Scan to pair: http://192.168.1.24:8790#pair=K7QF2M",
        ).assertExists()
    }

    // Without a code the QR is still worth having — it is how the address gets onto the phone at
    // all — but it must not claim to carry an enrolment it has not got.
    @Test
    fun withNoCodeItCarriesTheAddressAlone() = runComposeUiTest {
        setContent(setup(wide = true, code = null))
        onNodeWithContentDescription("Scan to open: http://192.168.1.24:8790").assertExists()
    }

    @Test
    fun aPhoneInPortraitIsNotShownAPictureOfWhereItAlreadyIs() = runComposeUiTest {
        setContent(setup(wide = false, code = "K7QF2M"))
        onNodeWithContentDescription(
            "Scan to pair: http://192.168.1.24:8790#pair=K7QF2M",
        ).assertDoesNotExist()
    }
}
