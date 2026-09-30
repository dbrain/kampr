package dev.kampr.terminal

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runDesktopComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.TerminalView
import kotlin.test.Test
import kotlin.test.assertTrue

private class FootIo : PaneIo {
    val claims = mutableListOf<Pair<Int, Int>>()
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
    override suspend fun claimMatch(paneId: String, cols: Int, rows: Int, grow: Boolean): Boolean {
        claims += cols to rows
        return true
    }
}

private class DeskTerminal(private val session: PaneSession, private val io: PaneIo) : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) =
        TerminalView(pane, session, io, modifier)

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Unit

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Unit
}

// A desk's pane is matched to the view, so its last row lands wherever the division of the window
// by the cell leaves it — flush with the edge on some heights. The status strip used to stand
// there; with it gone, and no gesture handle on a desktop window, nothing else will.
@OptIn(ExperimentalTestApi::class)
class DeskFootTest {
    @Test
    fun aMatchedPanesLastRowStopsShortOfTheBottomOfADeskWindowAtEveryHeight() {
        for (h in 1000..1017) {
            val io = FootIo()
            val session = PaneSession(Phone.PANE)
            val pane = PaneState(Phone.PANE, StyleTable())
            pane.applyReset(filling(pane, 94, 40))
            runDesktopComposeUiTest(1624, h) {
                setContent {
                    CompositionLocalProvider(
                        LocalTokens provides tokensFor(SoftTheme, TypeScale.Desk),
                        LocalPaneIo provides io,
                        LocalSafeArea provides SafeArea(top = 0.dp, bottom = 0.dp),
                        LocalConnectionStatus provides ConnectionStatus.Live("full"),
                    ) {
                        PaneScreenDesktop(
                            pane = pane,
                            info = null,
                            view = PaneView.Terminal,
                            surfaces = DeskTerminal(session, io),
                            readOnly = false,
                            onView = {},
                            modifier = Modifier.fillMaxSize(),
                        )
                    }
                }
                waitForIdle()
                val (cols, rows) = io.claims.last()
                pane.applyReset(filling(pane, cols, rows))
                waitForIdle()
                val bottom = rowBottom(pane, session, rows - 1)
                assertTrue(
                    bottom <= h.dp - 6.dp,
                    "a ${cols}x$rows pane matched to a ${h}dp window ends its last row at $bottom",
                )
            }
        }
    }
}

private fun filling(pane: PaneState, cols: Int, rows: Int) = ServerMsg.GridReset(
    pane = pane.id,
    cols = cols,
    rows = rows,
    rowsData = (0 until rows).map { RowDiff(it, listOf(Run(0, "line $it"))) },
    cursor = Cursor(0, rows - 1, true),
    links = emptyList(),
)
