package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.ClipboardManager
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneChrome
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.ui.keyboardInset
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.TerminalView
import kotlin.test.assertTrue

internal const val NOTES = "/home/dbrain/dev/kampr/notes.md"
internal const val SHOWN = "$ cat $NOTES"

// What `PaneScreen` measures its own header at and hands down, and the whole of the geometry this
// harness exists to reproduce: the header is painted *over* the terminal surface, so every dp
// above this line belongs to the bar and nothing the surface puts there can be seen or pressed.
internal val CHROME = 96.dp

internal class Route(
    private val answer: AttachmentBytes,
    override val readOnly: Boolean = false,
) : PaneIo {
    val asked = mutableListOf<String>()
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes {
        asked += id
        return answer
    }
}

@Suppress("DEPRECATION")
internal class Pasteboard : ClipboardManager {
    var held: AnnotatedString? = null
    override fun setText(annotatedString: AnnotatedString) {
        held = annotatedString
    }
    override fun getText(): AnnotatedString? = held
    override fun hasText(): Boolean = held != null
}

internal fun words(text: String) = AttachmentBytes.Ok(text.encodeToByteArray(), "text/plain")

// As tall as the desktop that made it, with the caret on the last line written. Both matter to
// where a tap lands. The default zoom is max(fit-width, fit-height), so an eight-row pane on a
// phone is blown up until barely nine columns are on screen and a column eight cells in is off
// the right edge — and how far off depends on the line height of whatever font the machine
// resolves for monospace, which is why that only ever failed on a runner. A full-height pane is
// width-fit instead, and the caret's row is the one row `caretFloor` guarantees is on screen.
internal fun paneShowing(vararg lines: String): PaneState {
    val pane = PaneState(Phone.PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = 94,
            rows = 40,
            rowsData = lines.mapIndexed { row, text -> RowDiff(row, listOf(Run(0, text))) },
            cursor = Cursor(lines.last().length, lines.lastIndex, true),
            links = emptyList(),
        ),
    )
    return pane
}

// The pane as a window of the given size holds it, with a clipboard a test can read back. The
// header is not composed — it belongs to `PaneScreen` — but the number it hands down is, because
// that number is the contract every floating surface on this screen has to honour.
@OptIn(ExperimentalTestApi::class)
internal fun ComposeUiTest.gridTerminal(
    pane: PaneState,
    session: PaneSession,
    io: PaneIo,
    width: Dp = 411.dp,
    height: Dp = 914.dp,
    bars: SafeArea = Phone.BARS,
    board: Pasteboard = Pasteboard(),
): Pasteboard {
    setContent {
        CompositionLocalProvider(
            LocalTokens provides Phone.tokens(),
            LocalPaneIo provides io,
            LocalSafeArea provides bars,
            LocalPaneChrome provides PaneChrome(CHROME),
            LocalClipboardManager provides board,
        ) {
            Box(Modifier.size(width, height).keyboardInset()) {
                Box(Modifier.fillMaxSize()) { TerminalView(pane, session, io) }
            }
        }
    }
    waitForIdle()
    return board
}

// A desk: wide and tall enough to be `Breakpoint.Desktop`, and with no system bars, which is what
// a browser reports. The notch a phone pays for is not what pushes this surface's chrome about.
internal val DESK_WIDTH = 1440.dp
internal val DESK_HEIGHT = 900.dp

@OptIn(ExperimentalTestApi::class)
internal fun ComposeUiTest.deskTerminal(pane: PaneState, session: PaneSession, io: PaneIo): Pasteboard =
    gridTerminal(pane, session, io, DESK_WIDTH, DESK_HEIGHT, SafeArea.None)

@OptIn(ExperimentalTestApi::class)
internal fun ComposeUiTest.tapCell(session: PaneSession, row: Int, col: Int): Offset {
    val grid = session.grid
    assertTrue(grid.cellWidth > 1f, "the grid has not been painted, so nothing is being tapped")
    val at = Offset(
        grid.originX + (col + 0.5f) * grid.cellWidth,
        grid.originY + (row + 0.5f) * grid.cellHeight,
    )
    // A point off the surface is delivered to nothing at all, so without this the test reads as
    // "the grid does not offer paths" when what happened is that the cell was never touched.
    val surface = onRoot().fetchSemanticsNode().size
    assertTrue(
        at.x >= 0f && at.y >= 0f && at.x < surface.width && at.y < surface.height,
        "cell $row,$col is painted at $at, outside the ${surface.width}x${surface.height} surface",
    )
    onRoot().performTouchInput {
        down(at)
        up()
    }
    waitForIdle()
    return at
}
