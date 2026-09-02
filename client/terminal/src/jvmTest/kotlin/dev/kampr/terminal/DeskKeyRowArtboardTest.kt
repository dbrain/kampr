package dev.kampr.terminal

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.platform.LocalHardKeyboard
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.PaneKeyRow
import dev.kampr.terminal.view.TerminalView
import java.io.File
import kotlin.test.Test

private const val PANE = "01JKAMPRNODE0000000000000/w4:p1"

private object DeskIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
    override fun info(paneId: String) = PaneInfo(
        id = PANE, nodeId = "01JKAMPRNODE0000000000000", workspace = "kampr", tab = "1",
        cwd = "~/dev/kampr", agent = "claude", agentStatus = "working", cols = 94, rows = 40,
        hasConversation = true,
    )
}

private val LINES = listOf(
    "dbrain@comingclean ~/dev/kampr $ cargo test --workspace",
    "",
    "   Compiling kampr-term v0.1.0 (crates/kampr-term)",
    "   Compiling kampr-core v0.1.0 (crates/kampr-core)",
    "    Finished `test` profile [unoptimized] in 21.8s",
    "",
    "running 214 tests",
    "test result: ok. 214 passed; 0 failed; 0 ignored",
    "",
    "dbrain@comingclean ~/dev/kampr $ vim docs/03-probe-log.md",
)

private fun deskPane(): PaneState {
    val pane = PaneState(PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = PANE,
            cols = 94,
            rows = 40,
            rowsData = LINES.mapIndexedNotNull { index, text ->
                text.takeIf { it.isNotEmpty() }?.let { RowDiff(index, listOf(Run(0, it))) }
            },
            cursor = Cursor(LINES.last().length, LINES.size - 1, true),
            links = emptyList(),
        ),
    )
    return pane
}

private class DeskSurfaces(private val session: PaneSession) : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) =
        TerminalView(pane, session, DeskIo, modifier)

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Unit

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) {
        val sink = InputSink(pane.id, DeskIo, session.latches)
        PaneKeyRow(
            session, sink, compact, enabled = true,
            modifier = modifier.onSizeChanged { session.keyRowHeight = it.height.toFloat() },
        )
    }
}

// 1280x800 dp is an Android tablet in landscape, which is the desktop breakpoint, and the pair of
// artboards is the whole of the change: the same window with and without a keyboard on it.
class DeskKeyRowArtboardTest {
    @Test
    fun theDesktopLayoutRendersWithAndWithoutTheKeyRow() {
        for (keyboard in listOf(false, true)) {
            val name = if (keyboard) "desk-with-keyboard" else "desk-no-keyboard"
            renderArtboard(1280.dp, 800.dp, SoftTheme, TypeScale.Desk, File(OUT, "$name.png")) {
                CompositionLocalProvider(
                    LocalPaneIo provides DeskIo,
                    LocalHardKeyboard provides keyboard,
                ) {
                    PaneScreenDesktop(
                        pane = deskPane(),
                        info = DeskIo.info(PANE),
                        view = PaneView.Terminal,
                        surfaces = DeskSurfaces(PaneSession(PANE)),
                        readOnly = false,
                        onView = {},
                    )
                }
            }
        }
    }
}
