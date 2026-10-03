package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaletteHost
import dev.kampr.terminal.input.drainInput
import dev.kampr.terminal.input.focusInput
import dev.kampr.terminal.view.TerminalView
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// The desk report: ctrl+K put the palette up over a terminal, and what was typed next went to the
// greyed-out pane underneath. The terminal's offscreen input holds the page's one focus slot and
// renews the claim every frame, so a Compose field asking for focus never got the keys.
@OptIn(ExperimentalTestApi::class)
class BrowserPaletteTest {
    @Test
    fun ctrlKOverADeskTerminalTakesTheKeyboardAwayFromThePane() = runComposeUiTest {
        pretendDesk()
        try {
            mainClock.autoAdvance = false
            val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
            val tokens = KamprTokens(SoftTheme, fonts, typography(fonts, SoftTheme.label, TypeScale.Phone))
            val session = PaneSession(BROWSER_PANE)
            setContent {
                CompositionLocalProvider(LocalTokens provides tokens, LocalPaneIo provides Hush) {
                    PaletteHost(items = { emptyList() }, onPick = {}) {
                        Box(Modifier.size(DESK.first, DESK.second)) {
                            Box(Modifier.fillMaxSize()) { TerminalView(shellPane(40, 3), session, Hush) }
                        }
                    }
                }
            }
            frames(30)
            assertTrue(inputFocused(), "the desk pane never held the keyboard, so there is nothing to take")

            chordKey("k", ctrl = true, meta = false, shift = false)
            frames(30)
            onNodeWithContentDescription("Search panes and places", useUnmergedTree = true).assertExists()
            assertFalse(inputFocused(), "the palette is up and the pane underneath still has the keyboard")
        } finally {
            stopPretending()
            focusInput(false)
            drainInput()
        }
    }
}
