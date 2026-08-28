package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
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

internal object HushIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
}

// A 1080x2400 phone, and a herdr pane as the node actually serves one. Named rather than inlined
// so an assertion can say "below the header" rather than "on screen somewhere". Everything here is
// under one object because the package is full of per-file `PANE`s and `shellPane`s.
internal object Phone {
    const val PANE = "01JKAMPRNODE0000000000000/w1:p1"

    // The header the pane screen measures and hands down.
    val HEADER = 96.dp
    val BARS = SafeArea(top = 32.dp, bottom = 46.dp)

    // Gboard. `bottom` is zero because the keyboard is drawn over the navigation bar —
    // `KeyboardFloor` is what takes it, and SafeAreaValueTest pins that.
    val KEYBOARD = SafeArea(top = 32.dp, bottom = 0.dp, ime = 320.dp)

    fun tokens() = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
        .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Phone)) }

    // The desktop's row count, a few lines of output at the top, the caret on the last of them,
    // and the whole rest of the grid blank. The caret is nowhere near the bottom of the grid,
    // which is the case a bottom-pinned surface gets wrong.
    fun shell(rows: Int = 40, caretRow: Int = 3): PaneState {
        val pane = PaneState(PANE, StyleTable())
        val lines = (0..caretRow).map { "[20:36:31 dbrain@comingclean kampr]$ line $it" }
        pane.applyReset(
            ServerMsg.GridReset(
                pane = PANE,
                cols = 94,
                rows = rows,
                rowsData = lines.mapIndexed { index, text -> RowDiff(index, listOf(Run(0, text))) },
                cursor = Cursor(lines.last().length, caretRow, true),
                links = emptyList(),
            ),
        )
        return pane
    }
}

// Composed the way the phone composes it: bars first, and whatever the caller does to them
// afterwards. The keyboard going up is a change to a surface that has already settled, which is
// the whole sequence — a pane composed with the keyboard already up never had a bottom to lose.
@OptIn(ExperimentalTestApi::class)
internal fun ComposeUiTest.phoneTerminal(
    pane: PaneState,
    session: PaneSession,
    width: Dp = 411.dp,
    height: Dp = 914.dp,
    io: PaneIo = HushIo,
): MutableState<SafeArea> {
    val bars = mutableStateOf(Phone.BARS)
    setContent {
        CompositionLocalProvider(
            LocalTokens provides Phone.tokens(),
            LocalPaneIo provides io,
            LocalSafeArea provides bars.value,
            LocalPaneChrome provides PaneChrome(Phone.HEADER),
        ) {
            // The shape the phone stacks: the app root pays the keyboard once, and the pane fills
            // what is left. Nothing inside knows the keyboard is there — which is the whole point,
            // and the reason the surface has to notice that it got shorter.
            Box(Modifier.size(width, height).keyboardInset()) {
                Box(Modifier.fillMaxSize()) { TerminalView(pane, session, io) }
            }
        }
    }
    waitForIdle()
    return bars
}
