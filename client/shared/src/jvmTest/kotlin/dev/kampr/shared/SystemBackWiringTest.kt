package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.InternalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.backhandler.LocalCompatNavigationEventDispatcherOwner
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppScaffold
import dev.kampr.shared.ui.AuthSurface
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.NoMosaic
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.Sheet
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull

private const val PANE = "01JNODE/w1:p1"

private const val HELLO_BACK =
    """{"build":"0.1.0","caps":{"conversation":true,"manage":true,"mesh":false,"push":true,""" +
        """"scrollback":true},"node_id":"01JNODE","node_name":"stub","protocol":1,"role":"full",""" +
        """"security":{"encrypted":true,"installable":false,"origin":"http://127.0.0.1",""" +
        """"passkeys":false,"push":true,"tier":0,"unencrypted_banner":false,"unlocks":[]},""" +
        """"t":"hello"}"""

// The report was a phone: back on a terminal closed Kampr instead of returning to the list. The
// mapping is `SystemBackTest`'s; this is the half that had gone missing entirely — nothing in the
// app claimed the gesture, so Android finished the activity. It has to be pressed through a real
// composition of the app's own scaffold, or the test passes with no handler in it at all.
@OptIn(ExperimentalTestApi::class, InternalComposeUiApi::class)
class SystemBackWiringTest {
    private fun ComposeUiTest.app(
        state: AppState,
        breakpoint: Breakpoint = Breakpoint.Portrait,
    ): SystemBackWindow {
        val window = SystemBackWindow()
        val size = if (breakpoint == Breakpoint.Desktop) 1280.dp to 900.dp else 411.dp to 914.dp
        setContent {
            CompositionLocalProvider(
                LocalTokens provides phoneTokens(),
                LocalSafeArea provides BARS,
                LocalCompatNavigationEventDispatcherOwner provides window,
            ) {
                Box(Modifier.size(size.first, size.second)) {
                    AppScaffold(
                        state = state,
                        breakpoint = breakpoint,
                        surfaces = BlankSurfaces,
                        mosaic = NoMosaic,
                        now = 0.0,
                        auth = auth(),
                        connectionStatus = ConnectionStatus.Live("full"),
                        deepLink = null,
                    )
                }
            }
        }
        waitForIdle()
        return window
    }

    private fun state(): Pair<AppState, CoroutineScope> {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore()
        store.accept(Wire.decode(HELLO_BACK) ?: error("undecodable hello"))
        val prefs = MemoryPrefs()
        prefs.set("endpoint", "http://127.0.0.1:8790")
        prefs.set("token", "tok")
        return AppState(scope, store, prefs, null) to scope
    }

    @Test
    fun theBackButtonOnATerminalReturnsToTheHerdInsteadOfClosingTheApp() {
        val (app, scope) = state()
        try {
            runComposeUiTest {
                app.openPane(PANE, PaneView.Terminal)
                val window = app(app)
                assertEquals(Screen.Pane(PANE, PaneView.Terminal), app.screen)

                window.press()
                waitForIdle()
                assertEquals(Screen.Herd, app.screen, "back on a pane has to lead to the list")
            }
        } finally {
            scope.cancel()
        }
    }

    // The herd is where back is allowed to close Kampr, so nothing there may claim the gesture —
    // an app that cannot be left by its own back button is the other half of this defect. Both
    // directions, because a scaffold that claimed nothing anywhere would pass the first half.
    @Test
    fun theHerdLetsTheGestureThroughAndAPaneDoesNot() {
        val (app, scope) = state()
        try {
            runComposeUiTest {
                val window = app(app)
                assertEquals(Screen.Herd, app.screen)
                assertEquals(false, window.claimed, "the herd claims back, so Kampr cannot be left")

                app.openPane(PANE, PaneView.Terminal)
                waitForIdle()
                assertEquals(true, window.claimed, "a pane leaves back to the platform")
            }
        } finally {
            scope.cancel()
        }
    }

    // Everything a sheet acts on is behind it, so the screen underneath has not been left yet.
    @Test
    fun backClosesTheSheetBeforeItLeavesTheScreenUnderIt() {
        val (app, scope) = state()
        try {
            runComposeUiTest {
                app.openPane(PANE, PaneView.Terminal)
                app.openSheet(Sheet.Actions(PANE))
                val window = app(app)
                assertNotNull(app.sheet, "the sheet is up")

                window.press()
                waitForIdle()
                assertNull(app.sheet, "back has to close the sheet first")
                assertEquals(
                    Screen.Pane(PANE, PaneView.Terminal),
                    app.screen,
                    "and must not take the screen behind it with the sheet",
                )

                window.press()
                waitForIdle()
                assertEquals(Screen.Herd, app.screen, "the next press is the one that leaves the pane")
            }
        } finally {
            scope.cancel()
        }
    }

    // `BackHandler` is the one API in this app that *errors* rather than degrading when the
    // platform has not provided a dispatcher — a white screen, not a dead gesture. Every other
    // test here hands it one, so none of them would see that. Desktop and the browser share the
    // skiko composition root that provides it, so this is the same code path the web app takes.
    @Test
    fun theSurfaceProvidesADispatcherWithoutBeingHandedOne() {
        val (app, scope) = state()
        try {
            runComposeUiTest {
                app.openPane(PANE, PaneView.Terminal)
                setContent {
                    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalSafeArea provides BARS) {
                        Box(Modifier.size(411.dp, 914.dp)) {
                            AppScaffold(
                                state = app,
                                breakpoint = Breakpoint.Portrait,
                                surfaces = BlankSurfaces,
                                mosaic = NoMosaic,
                                now = 0.0,
                                auth = auth(),
                                connectionStatus = ConnectionStatus.Live("full"),
                                deepLink = null,
                            )
                        }
                    }
                }
                waitForIdle()
                assertEquals(Screen.Pane(PANE, PaneView.Terminal), app.screen)
            }
        } finally {
            scope.cancel()
        }
    }

    // The settings ladder, walked back up one rung at a time rather than out of the app.
    @Test
    fun backWalksTheSettingsLadderUpwards() {
        val (app, scope) = state()
        try {
            runComposeUiTest {
                app.go(Screen.Appearance)
                val window = app(app)

                window.press()
                waitForIdle()
                assertEquals(Screen.Setup, app.screen)

                window.press()
                waitForIdle()
                assertEquals(Screen.Herd, app.screen, "the settings tab's root leads to the herd's")
            }
        } finally {
            scope.cancel()
        }
    }
}

private fun auth() = AuthSurface(
    setup = null,
    devices = emptyList(),
    currentDeviceId = null,
    pairingCode = null,
    failure = null,
    onPairingCode = {},
    onRevoke = {},
    onRenew = {},
    onDismissFailure = {},
)
