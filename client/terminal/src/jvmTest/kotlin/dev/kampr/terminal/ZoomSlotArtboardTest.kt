package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.view.ZoomButton
import java.io.File
import kotlin.test.Test

private const val PANE = "01JKAMPRNODE0000000000000/w3:p1"

private val INFO = PaneInfo(
    id = PANE, nodeId = "01JKAMPRNODE0000000000000", workspace = "kampr", tab = "1",
    cwd = "~/dev/kampr", agent = "claude", agentStatus = "blocked", cols = 94, rows = 40,
    hasConversation = true,
)

private object Io : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
    override fun info(paneId: String) = INFO
}

private object Manage : ManageIo {
    override val enabled = true
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
}

// The real control, because the slot is whatever it measures: the header is not being shown a 40 dp
// stand-in here, it is being shown the button an operator taps.
private class RealZoom(private val session: PaneSession) : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)
    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Unit
    @Composable override fun Zoom(pane: PaneState, modifier: Modifier) = ZoomButton(session, modifier)
}

private class Board(
    val name: String,
    val width: Dp,
    val height: Dp,
    val density: Float,
    val desktop: Boolean = false,
    val landscape: Boolean = false,
    val crowded: Boolean = false,
)

// Cropped to the chrome: the surfaces are blank, and what is being looked at is whether the header
// reads the same in both views or has a hole in it where the control used to be.
private val BOARDS = listOf(
    Board("zoom-slot-portrait", 411.dp, 190.dp, 2f),
    Board("zoom-slot-portrait-crowded", 360.dp, 190.dp, 3f, crowded = true),
    Board("zoom-slot-landscape", 914.dp, 110.dp, 2f, landscape = true),
    Board("zoom-slot-landscape-crowded", 740.dp, 110.dp, 2f, landscape = true, crowded = true),
    Board("zoom-slot-desktop", 1280.dp, 120.dp, 2f, desktop = true),
)

class ZoomSlotArtboardTest {
    @Test
    fun theHeaderRendersInBothViewsOnEveryLayoutThatCarriesAZoomControl() {
        for (board in BOARDS) {
            for (view in listOf(PaneView.Terminal, PaneView.Conversation)) {
                val suffix = if (view == PaneView.Conversation) "conversation" else "terminal"
                renderArtboard(
                    board.width, board.height, SoftTheme, TypeScale.Phone,
                    File(OUT, "${board.name}-$suffix.png"), density = Density(board.density),
                ) {
                    val pane = PaneState(PANE, StyleTable())
                    val surfaces = RealZoom(PaneSession(PANE))
                    CompositionLocalProvider(LocalPaneIo provides Io, LocalManage provides Manage) {
                        if (board.desktop) {
                            PaneScreenDesktop(pane, INFO, view, surfaces, board.crowded, {}, {})
                        } else {
                            PaneScreenMobile(pane, INFO, view, surfaces, board.landscape, board.crowded, {}, {}, {})
                        }
                    }
                }
            }
        }
    }
}
