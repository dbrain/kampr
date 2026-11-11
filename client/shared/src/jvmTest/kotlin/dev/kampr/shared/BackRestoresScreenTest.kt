package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.InternalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.navigationevent.compose.LocalNavigationEventDispatcherOwner
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppScaffold
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.AuthSurface
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.FilesNav
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.NoMosaic
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.action
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val NODE = "01JNODE"

private const val HELLO_KEPT =
    """{"t":"hello","protocol":1,"node_id":"$NODE","node_name":"comingclean","build":"test",""" +
        """"role":"full","caps":{"manage":true},"security":{"tier":0,"passkeys":false}}"""

private val PANES = (0 until 40).map { "$NODE/w$it:p1" }

private val HERD_FRAME = """{"t":"herd","nodes":[{"id":"$NODE","name":"comingclean","kind":"local"}],"panes":[""" +
    PANES.mapIndexed { i, id -> """{"id":"$id","node_id":"$NODE","workspace":"work-$i","cwd":"/home/u/w$i"}""" }.joinToString(",") +
    "]}"

private val DIR = FilesPlace.Dir(NODE, PANES[0], "/home/u")
private val FILE = FilesPlace.File(NODE, PANES[0], "/home/u/notes.md")

// Stands in for a files view: a count only `rememberSaveable` can carry across being covered.
private object CountingFiles : PaneSurfaces by BlankSurfaces {
    @Composable
    override fun Files(place: FilesPlace, nav: FilesNav, modifier: Modifier) {
        var taps by rememberSaveable { mutableIntStateOf(0) }
        Box(modifier) { BasicText("taps $taps", Modifier.action("Tap ${place.anchor}", { taps++ })) }
    }
}

private val VERTICAL = SemanticsMatcher("scrolls vertically") { node ->
    node.config.getOrNull(SemanticsProperties.VerticalScrollAxisRange)?.let { it.maxValue() > 0f } == true
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.scrolled(): Float = onNode(VERTICAL).fetchSemanticsNode().config[SemanticsProperties.VerticalScrollAxisRange].value()

private fun SemanticsNodeInteraction.scrollBy(px: Float) = performSemanticsAction(SemanticsActions.ScrollBy) { it(0f, px) }

// The report: going deeper and coming back with the system back left the screen underneath as if
// it had never been seen. Pressed through the app's own scaffold and its own back handler, since a
// view that keeps its state alone proves nothing about the stack around it.
@OptIn(ExperimentalTestApi::class, InternalComposeUiApi::class)
class BackRestoresScreenTest {
    private fun withApp(
        height: Dp = 914.dp,
        surfaces: PaneSurfaces = BlankSurfaces,
        status: ConnectionStatus = ConnectionStatus.Live("full"),
        block: ComposeUiTest.(AppState, SystemBackWindow) -> Unit,
    ) {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore()
        store.accept(Wire.decode(HELLO_KEPT) ?: error("undecodable hello"))
        store.accept(Wire.decode(HERD_FRAME) ?: error("undecodable herd"))
        val prefs = MemoryPrefs()
        prefs.set("endpoint", "http://127.0.0.1:8790")
        prefs.set("token", "tok")
        val app = AppState(scope, store, prefs, null)
        try {
            runComposeUiTest {
                val window = SystemBackWindow()
                setContent {
                    CompositionLocalProvider(
                        LocalTokens provides phoneTokens(),
                        LocalSafeArea provides BARS,
                        LocalNavigationEventDispatcherOwner provides window,
                    ) {
                        Box(Modifier.size(411.dp, height)) {
                            AppScaffold(app, Breakpoint.Portrait, surfaces, NoMosaic, 0.0, AUTH, status, null)
                        }
                    }
                }
                waitForIdle()
                block(app, window)
            }
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun backFromAFileRestoresTheFolderUnderItAndAFolderOpenedAgainStartsFresh() = withApp(surfaces = CountingFiles) { app, window ->
        app.open(DIR)
        waitForIdle()
        repeat(3) { onNodeWithContentDescription("Tap ${DIR.anchor}").performClick() }
        onNodeWithText("taps 3").assertExists()

        app.open(FILE)
        waitForIdle()
        onNodeWithText("taps 0").assertExists()

        window.press()
        waitForIdle()
        assertEquals(listOf<FilesPlace>(DIR), app.files)
        onNodeWithText("taps 3").assertExists()

        window.press()
        waitForIdle()
        app.open(DIR)
        waitForIdle()
        onNodeWithText("taps 0").assertExists()
    }

    @Test
    fun backFromAPaneRestoresTheHerdsScroll() = withApp { app, window ->
        onNode(VERTICAL).scrollBy(900f)
        waitForIdle()
        val before = scrolled()
        assertTrue(before > 0f, "the herd scrolled")

        app.openPane(PANES[30], PaneView.Terminal)
        waitForIdle()
        window.press()
        waitForIdle()

        assertEquals(Screen.Herd, app.screen)
        assertEquals(before, scrolled())
    }

    @Test
    fun backFromAPaneOpenedOffFleetReturnsToFleet() = withApp { app, window ->
        app.go(Screen.Fleet)
        waitForIdle()
        app.openPane(PANES[3], PaneView.Terminal)
        waitForIdle()
        app.openPane(PANES[4], PaneView.Terminal)
        waitForIdle()
        window.press()
        waitForIdle()
        assertEquals(Screen.Fleet, app.screen)

        window.press()
        waitForIdle()
        app.openPane(PANES[3], PaneView.Terminal)
        waitForIdle()
        window.press()
        waitForIdle()
        assertEquals(Screen.Herd, app.screen)
    }

    @Test
    fun backFromDevicesRestoresTheSetupScroll() = withApp(height = 520.dp) { app, window ->
        app.go(Screen.Setup)
        waitForIdle()
        onNode(VERTICAL).scrollBy(400f)
        waitForIdle()
        val before = scrolled()
        assertTrue(before > 0f, "setup scrolled")

        app.go(Screen.Devices)
        waitForIdle()
        window.press()
        waitForIdle()

        assertEquals(Screen.Setup, app.screen)
        assertEquals(before, scrolled())

        app.go(Screen.Herd)
        waitForIdle()
        app.go(Screen.Setup)
        waitForIdle()
        assertEquals(0f, scrolled(), "setup reached afresh from the herd starts at its top")
    }

    @Test
    fun backFromDevicesKeepsWhatWasTypedIntoSetup() = withApp(status = ConnectionStatus.Connecting) { app, window ->
        app.go(Screen.Setup)
        waitForIdle()
        onNodeWithContentDescription("Pairing code, only when pairing").performTextInput("ABCD")
        waitForIdle()

        app.go(Screen.Devices)
        waitForIdle()
        window.press()
        waitForIdle()

        val code = onNodeWithContentDescription("Pairing code, only when pairing").fetchSemanticsNode()
        assertEquals("ABCD", code.config[SemanticsProperties.EditableText].text)
    }
}

private val AUTH = AuthSurface(
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
