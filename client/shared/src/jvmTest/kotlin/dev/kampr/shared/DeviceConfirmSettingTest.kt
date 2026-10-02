package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppPaneIo
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.SetupScreen
import dev.kampr.shared.wire.Security
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private const val TOGGLE = "Check destructive commands."

// One switch for the device, so a pane nobody has touched follows it rather than each pane having
// to be opted in by hand; the pane's own answer still wins, which `SubmitGuardTest` holds.
@OptIn(ExperimentalTestApi::class)
class DeviceConfirmSettingTest {
    @Test
    fun theSettingsSwitchIsWhatEveryPaneOnThisDeviceFollowsAndItSurvivesAReload() = runComposeUiTest {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val prefs = MemoryPrefs()
        val state = AppState(scope, prefs = prefs, derived = null)
        assertFalse(AppPaneIo(state).confirmsByDefault, "a fresh device checks destructive commands")

        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(420.dp, 1600.dp)) {
                    SetupScreen(
                        status = null, security = Security(), running = true, endpoint = null,
                        nodes = emptyList(), pairingCode = null, pairingError = null,
                        onConnect = {}, onPairingCode = {}, onDevices = {}, onAppearance = {}, onNotifications = {},
                        confirmRisky = state.confirmRisky,
                        onConfirmRisky = state::confirmRisky,
                    )
                }
            }
        }
        onNodeWithContentDescription(TOGGLE, substring = true).performScrollTo().performClick()
        waitForIdle()

        assertTrue(AppPaneIo(state).confirmsByDefault, "the switch did not reach the panes")
        assertTrue(AppPaneIo(AppState(scope, prefs = prefs, derived = null)).confirmsByDefault, "the switch was forgotten on reload")
        assertEquals("1", prefs.get("confirm.risky"))
        scope.cancel()
    }
}
