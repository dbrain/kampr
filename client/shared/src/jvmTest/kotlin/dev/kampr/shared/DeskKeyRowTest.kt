package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SkikoComposeUiTest
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.v2.runSkikoComposeUiTest
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.platform.LocalHardKeyboard
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.named
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test

private const val PANE_ID = "01JNODE/w1:p1"
private const val KEY_ROW = "Key row probe"

private val INFO = PaneInfo(
    id = PANE_ID,
    nodeId = "01JNODE",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = "idle",
    cols = 94,
    rows = 40,
    hasConversation = true,
)

private object KeyRowProbe : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) =
        Box(modifier.size(200.dp, 90.dp).named(KEY_ROW))
    @Composable override fun Zoom(pane: PaneState, modifier: Modifier) = Box(modifier.size(40.dp))
}

@OptIn(ExperimentalTestApi::class)
private fun window(width: Dp, height: Dp, block: SkikoComposeUiTest.() -> Unit) =
    runSkikoComposeUiTest(Size(width.value, height.value), Density(1f)) { block() }

@Composable
private fun Desk(keyboard: Boolean, view: PaneView = PaneView.Terminal) {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalHardKeyboard provides keyboard) {
        Box(Modifier.fillMaxSize()) {
            PaneScreenDesktop(
                pane = PaneState(PANE_ID, StyleTable()),
                info = INFO,
                view = view,
                surfaces = KeyRowProbe,
                readOnly = false,
                onView = {},
                modifier = Modifier.fillMaxSize(),
            )
        }
    }
}

// The report: an Android tablet in landscape is 1280×800 dp, which is the desktop breakpoint, and
// the desktop header composes no key row — so a device with no keys at all had no Escape, no Ctrl,
// no arrows and no way to answer a prompt. The breakpoint was never the question; whether there is
// a keyboard is.
@OptIn(ExperimentalTestApi::class)
class DeskKeyRowTest {
    @Test
    fun aTabletOnTheDesktopBreakpointWithNoKeyboardStillGetsItsEscapeKey() = window(1280.dp, 800.dp) {
        setContent { Desk(keyboard = false) }
        waitForIdle()
        onNodeWithContentDescription(KEY_ROW).assertIsDisplayed()
    }

    @Test
    fun aDeskWithAKeyboardIsNotGivenAStripOfTheKeysItIsAlreadyHolding() = window(1280.dp, 800.dp) {
        setContent { Desk(keyboard = true) }
        waitForIdle()
        onNodeWithContentDescription(KEY_ROW).assertDoesNotExist()
    }

    // The report, verbatim: the caps "stay for a while then go away weirdly". Every platform's
    // reading can move mid-session — Android's on a configuration change, a browser's because it
    // is a guess — and the two directions are not the same size. A spare row on a desk is clutter;
    // a row that leaves takes Escape, the arrows and every latch off the screen of whoever was
    // reaching for them. So nothing may take it back once it is up.
    @Test
    fun aKeyboardNoticedMidSessionDoesNotTakeTheRowOffTheScreenUnderneathAnOperator() = window(1280.dp, 800.dp) {
        var attached by mutableStateOf(false)
        setContent { Desk(keyboard = attached) }
        waitForIdle()
        onNodeWithContentDescription(KEY_ROW).assertIsDisplayed()
        attached = true
        waitForIdle()
        onNodeWithContentDescription(KEY_ROW).assertIsDisplayed()
    }

    // The direction that must still work, and the reason this is a running rule rather than one
    // reading frozen at composition: undocking a tablet from its keyboard case is the moment the
    // row exists for, and it happens in the composition that is already on screen.
    @Test
    fun aKeyboardTakenAwayMidSessionBringsTheRowBackWithoutRestartingTheApp() = window(1280.dp, 800.dp) {
        var attached by mutableStateOf(true)
        setContent { Desk(keyboard = attached) }
        waitForIdle()
        onNodeWithContentDescription(KEY_ROW).assertDoesNotExist()
        attached = false
        waitForIdle()
        onNodeWithContentDescription(KEY_ROW).assertIsDisplayed()
    }

    // The transcript renders its own composer, and a key row over it is a second one stacked on the
    // first — which is why the mobile layout stands its own down there too.
    @Test
    fun theTranscriptCarriesNoKeyRowEvenWithNoKeyboardAnywhere() = window(1280.dp, 800.dp) {
        setContent { Desk(keyboard = false, view = PaneView.Conversation) }
        waitForIdle()
        onNodeWithContentDescription(KEY_ROW).assertDoesNotExist()
    }

    // The two layouts that already had the row keep it unconditionally: a phone's key row is not a
    // fallback for a missing keyboard, it is how a phone types Ctrl-C, and a Bluetooth keyboard on
    // a phone does not take that away.
    @Test
    fun aPhoneKeepsItsKeyRowWhateverAKeyboardIsDoing() {
        for (landscape in listOf(false, true)) {
            val size = if (landscape) 914.dp to 411.dp else 411.dp to 914.dp
            window(size.first, size.second) {
                setContent {
                    CompositionLocalProvider(
                        LocalTokens provides phoneTokens(),
                        LocalHardKeyboard provides true,
                    ) {
                        Box(Modifier.fillMaxSize()) {
                            PaneScreenMobile(
                                pane = PaneState(PANE_ID, StyleTable()),
                                info = INFO,
                                view = PaneView.Terminal,
                                surfaces = KeyRowProbe,
                                landscape = landscape,
                                readOnly = false,
                                onBack = {},
                                onView = {},
                                modifier = Modifier.fillMaxSize(),
                            )
                        }
                    }
                }
                waitForIdle()
                onNodeWithContentDescription(KEY_ROW).assertIsDisplayed()
            }
        }
    }
}
