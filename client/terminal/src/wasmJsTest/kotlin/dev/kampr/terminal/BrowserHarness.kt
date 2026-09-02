package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
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

internal object Hush : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
}

internal const val BROWSER_PANE = "01JKAMPRNODE0000000000000/w1:p1"

internal val DESK = 1600.dp to 900.dp
internal val PHONE = 411.dp to 914.dp

internal fun shellPane(rows: Int, caretRow: Int): PaneState {
    val pane = PaneState(BROWSER_PANE, StyleTable())
    val lines = (0..caretRow).map { "[20:36:31 dbrain@comingclean kampr]$ line $it" }
    pane.applyReset(
        ServerMsg.GridReset(
            pane = BROWSER_PANE,
            cols = 94,
            rows = rows,
            rowsData = lines.mapIndexed { index, text -> RowDiff(index, listOf(Run(0, text))) },
            cursor = Cursor(lines.last().length, caretRow, true),
            links = emptyList(),
        ),
    )
    return pane
}

// **`waitForIdle` cannot be used in this harness, and that is a measurement rather than a
// preference.** With the auto-advancing clock a real `TerminalView` never reaches idle in
// ChromeHeadless: the frame loop spins inside `setContent` and blocks the browser's main thread
// outright — a `setInterval` watchdog installed before it never fires once, and karma loses the
// socket to a ping timeout rather than to a slow test. So the clock is driven by hand here, which
// is also what lets a caret sweep stay inside `CARET_SETTLE_MS` on purpose.
@OptIn(ExperimentalTestApi::class)
internal fun ComposeUiTest.frames(count: Int) {
    repeat(count) { mainClock.advanceTimeByFrame() }
}

@OptIn(ExperimentalTestApi::class)
internal fun ComposeUiTest.browserTerminal(
    pane: PaneState,
    session: PaneSession,
    size: Pair<Dp, Dp>,
) {
    mainClock.autoAdvance = false
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    val tokens = KamprTokens(SoftTheme, fonts, typography(fonts, SoftTheme.label, TypeScale.Phone))
    setContent {
        CompositionLocalProvider(
            LocalTokens provides tokens,
            LocalPaneIo provides Hush,
            LocalSafeArea provides SafeArea(top = 32.dp, bottom = 46.dp),
            LocalPaneChrome provides PaneChrome(96.dp),
        ) {
            Box(Modifier.size(size.first, size.second).keyboardInset()) {
                Box(Modifier.fillMaxSize()) { TerminalView(pane, session, Hush) }
            }
        }
    }
    frames(SETTLE_FRAMES)
}

// Long enough for the zoom to be adopted, the font advance to be re-probed and the opening caret
// reading to settle: 30 frames is 480 ms of virtual time against a 200 ms settle.
private const val SETTLE_FRAMES = 30

// One repaint step. Three frames is 48 ms — a fifth of `CARET_SETTLE_MS` — so a sweep of them is a
// caret that has not stopped anywhere, which is exactly what a full-screen redraw is.
@OptIn(ExperimentalTestApi::class)
internal fun ComposeUiTest.caretTo(pane: PaneState, row: Int, hold: Int = 3) {
    pane.applyPatch(
        ServerMsg.GridPatch(
            pane = BROWSER_PANE,
            rows = listOf(RowDiff(row, listOf(Run(0, "  redrawn row $row")))),
            cursor = Cursor(0, row, true),
            links = emptyList(),
        ),
    )
    frames(hold)
}
