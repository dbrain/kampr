package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithText
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
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

private val NODE = NodeInfo(id = "01JFRONT", name = "front", kind = "local")
private const val ANYWHERE = "Reach it from anywhere"
private const val HOSTNAME = "A hostname and certificate"

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.ladder(security: Security, onPasskeys: (() -> Unit)? = null): List<String> {
    setContent {
        CompositionLocalProvider(LocalTokens provides tokens()) {
            Box(Modifier.size(420.dp, 1400.dp)) {
                SetupScreen(
                    status = null,
                    security = security,
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
                    onPasskeys = onPasskeys,
                )
            }
        }
    }
    waitForIdle()
    return listOf(ANYWHERE, HOSTNAME).filter {
        onAllNodesWithText(it, useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }
}

// `security.tier` only ever carries 0 or 1 — it is `u8::from(passkeys)` and nothing else — so the
// rung guarded by `tier < 3` was shown to every operator for ever, including the ones who had
// already done the thing it asks for.
@OptIn(ExperimentalTestApi::class)
class SetupLadderTest {
    @Test
    fun anOriginThatCannotTakeAPasskeyIsStillAskedToBecomeReachable() = runComposeUiTest {
        val rungs = ladder(Security(tier = 0))
        assertTrue(HOSTNAME in rungs, "the ladder lost the rung that says what a hostname buys")
        assertTrue(ANYWHERE in rungs, "an ip:port origin was never offered the way off it")
    }

    @Test
    fun anOriginThatAlreadyTakesPasskeysIsNotAskedToBecomeReachableAgain() = runComposeUiTest {
        val rungs = ladder(Security(tier = 1, passkeys = true))
        assertFalse(HOSTNAME in rungs)
        assertFalse(ANYWHERE in rungs, "a node already reachable at a name was still nagged to become one")
    }

    // The socket may not be up yet, and `/api/node` answers the same question without a token —
    // which is why the passkey rung reads that rather than `security` alone. Both rungs are about
    // the same fact and must not disagree with each other.
    @Test
    fun aNodeWhoseSocketIsNotUpYetIsReadTheSameWayByBothRungs() = runComposeUiTest {
        val rungs = ladder(Security(tier = 0), onPasskeys = {})
        assertFalse(HOSTNAME in rungs)
        assertFalse(ANYWHERE in rungs)
    }
}
