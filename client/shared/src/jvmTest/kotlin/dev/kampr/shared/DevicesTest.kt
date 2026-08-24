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
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.DeviceRecord
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.DevicesScreen
import kotlinx.serialization.json.Json
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), content = content)
}

private const val NOW = 1_760_000_000_000.0
private val json = Json { ignoreUnknownKeys = true; isLenient = true }

private fun device(body: String) = json.decodeFromString(DeviceRecord.serializer(), body)

// The rows the node serves, verbatim: it returns revoked and expired ones too, and `active` is
// its own answer about its own clock.
private val PHONE = device(
    """{"id":"d1","name":"pixel","role":"full","expires_at":1762000000,"active":true}"""
)
private val LAPSED = device(
    """{"id":"d2","name":"old tablet","role":"readonly","expires_at":1759000000,"active":false}"""
)
private val GONE = device(
    """{"id":"d3","name":"stolen laptop","role":"full","revoked_at":1759500000,"active":false}"""
)

private fun screen(
    devices: List<DeviceRecord>,
    onRevoke: (String) -> Unit = {},
    onRenew: (String) -> Unit = {},
): @Composable () -> Unit = {
    Themed {
        Box(Modifier.size(420.dp, 900.dp)) {
            DevicesScreen(devices, currentId = null, now = NOW, onBack = {}, onRevoke = onRevoke, onRenew = onRenew)
        }
    }
}

// A Tier-0 token expires by design, and this screen counted an expired one as paired and offered
// it a Revoke — telling the operator a device still has access that the node stopped honouring.
@OptIn(ExperimentalTestApi::class)
class DevicesTest {
    @Test
    fun anExpiredDeviceIsNotListedAsPaired() = runComposeUiTest {
        setContent(screen(listOf(PHONE, LAPSED)))
        waitForIdle()
        onNodeWithContentDescription("Revoke pixel", substring = true).assertExists()
        assertTrue(
            onAllNodesWithContentDescription("Revoke old tablet", substring = true).fetchSemanticsNodes().isEmpty(),
            "an expired device was offered a Revoke, which says it still has access",
        )
        assertTrue(
            onAllNodesWithText("expired", substring = true, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty(),
            "an expired device was drawn exactly like a paired one",
        )
    }

    @Test
    fun anExpiredDeviceIsOfferedTheRenewTheNodeAlreadyServes() = runComposeUiTest {
        val renewed = mutableListOf<String>()
        setContent(screen(listOf(PHONE, LAPSED), onRenew = { renewed += it }))
        waitForIdle()
        onNodeWithContentDescription("Renew old tablet", substring = true).performClick()
        waitForIdle()
        assertEquals(listOf("d2"), renewed)
    }

    // Additive on the wire: a node that has never heard of `active` sends the rows raw, and this
    // client must read them exactly as it did before.
    @Test
    fun aNodeThatSaysNothingAboutExpiryStillHasItsDevicesListedAsPaired() = runComposeUiTest {
        val silent = device("""{"id":"d9","name":"laptop","role":"full","expires_at":1759000000}""")
        assertTrue(silent.active, "a row with no `active` was read as saying something")
        assertFalse(GONE.active)
        setContent(screen(listOf(silent)))
        waitForIdle()
        onNodeWithContentDescription("Revoke laptop", substring = true).assertExists()
    }

    // Revoked is not expired: one is a decision somebody made and the other is a clock running
    // out, and only the second is worth putting in front of the operator with a way back.
    @Test
    fun aRevokedDeviceIsNotListedAtAll() = runComposeUiTest {
        setContent(screen(listOf(PHONE, GONE)))
        waitForIdle()
        assertTrue(
            onAllNodesWithText("stolen laptop", substring = true, useUnmergedTree = true).fetchSemanticsNodes().isEmpty(),
            "a revoked device was listed",
        )
    }
}
