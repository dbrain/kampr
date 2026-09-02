package dev.kampr.terminal

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.runDesktopComposeUiTest
import androidx.compose.ui.unit.Dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import kotlin.test.Test
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

private val ROOM_INFO = PaneInfo(
    id = Phone.PANE, nodeId = "01JKAMPRNODE0000000000000", workspace = "kampr", tab = "1",
    cwd = "~/dev/kampr", agent = "claude", agentStatus = "working", cols = 94, rows = 40,
    hasConversation = true,
)

private object RoomIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
    override fun info(paneId: String) = ROOM_INFO
}

private object RoomManage : ManageIo {
    override val enabled = true
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
}

// The real surfaces, with the two edges of the room written down as they are laid out: the header
// is the number the pane screen measures and hands the terminal, and the key row is where the
// bottom chrome actually starts. Neither can be read off a semantics node.
private class Measured(private val real: TerminalSurfaces = TerminalSurfaces()) : PaneSurfaces {
    var headerBottom: Dp? = null
    var keyRowTop: Dp? = null

    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) {
        headerBottom = LocalPaneChrome.current?.top
        real.Terminal(pane, info, modifier)
    }

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) =
        real.Conversation(pane, info, modifier)

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) = real.Zoom(pane, modifier)

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) {
        val density = LocalDensity.current
        real.KeyRow(
            pane, compact,
            modifier.onGloballyPositioned { keyRowTop = with(density) { it.positionInRoot().y.toDp() } },
        )
    }
}

// The sheet was sized and placed against the whole window rather than against the room the pane
// screen actually leaves it, so on a 1080x2410 phone it opened 686 dp tall into a 508 dp gap: its
// top 189 dp — the Zoom line and the whole minimap — lay behind the pane header, and scrolling
// could not reach them because it was the viewport that was under the header, not the content.
//
// Composed through `PaneScreenMobile` with the real key row rather than through `phoneTerminal`,
// which is what every zoom test near this one uses: that harness composes `TerminalView` alone, so
// there is no header above the sheet and no key row below it, and a sheet that overruns both is in
// perfectly good health there.
@OptIn(ExperimentalTestApi::class)
class ZoomSheetRoomTest {
    @Test
    fun everyControlInTheSheetOpensBetweenThePaneHeaderAndTheKeyRow() {
        // Portrait is the reported one; landscape is where the room is 150 dp and a sheet that
        // measures itself against the window is out by more than the room is worth.
        forPhone(393, 876, landscape = false)
        forPhone(876, 393, landscape = true)
    }

    private fun forPhone(width: Int, height: Int, landscape: Boolean) =
        runDesktopComposeUiTest(width, height) {
            val surfaces = Measured()
            setContent {
                CompositionLocalProvider(
                    LocalTokens provides Phone.tokens(),
                    LocalPaneIo provides RoomIo,
                    LocalManage provides RoomManage,
                    LocalSafeArea provides Phone.BARS,
                ) {
                    PaneScreenMobile(
                        Phone.shell(), ROOM_INFO, PaneView.Terminal, surfaces, landscape, false,
                        onBack = {}, onView = {}, modifier = Modifier.fillMaxSize(),
                    )
                }
            }
            waitForIdle()
            onNodeWithContentDescription("Zoom, currently", substring = true).performClick()
            waitForIdle()

            val top = assertNotNull(surfaces.headerBottom, "the pane header never measured itself")
            val bottom = assertNotNull(surfaces.keyRowTop, "the key row never laid itself out")
            val where = if (landscape) "landscape" else "portrait"
            for ((what, node) in sheetControls()) {
                node.performScrollTo()
                waitForIdle()
                val at = node.getUnclippedBoundsInRoot()
                assertTrue(
                    at.top >= top && at.bottom <= bottom,
                    "in $where the sheet's $what sits at ${at.top}..${at.bottom}, outside the " +
                        "$top..$bottom the pane screen leaves between the header and the key row",
                )
            }
        }

    private fun ComposeUiTest.sheetControls(): List<Pair<String, SemanticsNodeInteraction>> = listOf(
        "first line" to onNodeWithText("pinch to adjust", substring = true),
        "fit-width preset" to onNodeWithContentDescription("Fit width", substring = true),
        "last toggle" to onNodeWithContentDescription("Check destructive commands", substring = true),
        "closing note" to onNodeWithText("Zoom is yours alone", substring = true),
    )
}
