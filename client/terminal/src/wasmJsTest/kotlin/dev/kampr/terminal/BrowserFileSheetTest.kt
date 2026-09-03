package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneChrome
import dev.kampr.shared.ui.SafeArea
import dev.kampr.terminal.file.Peeked
import dev.kampr.terminal.view.FileSheet
import kotlin.test.Test
import kotlin.test.assertTrue

private const val PATH = "/home/dbrain/dev/kampr/notes.md"
private const val BODY = "the whole file\nand its second line"

// What a browser reports, which is the measurement the desk report turned on: no notch, no
// gesture handle, no system bars at all — so `safe.top` is zero on this platform and cannot be
// what holds a sheet's controls clear of anything. The pane header can, and does: `PaneScreen`
// paints it *over* the terminal surface and hands its measured height down as `PaneChrome`.
private val DESK_CHROME = 64.dp

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.sheet(chromeTop: Dp) {
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    val tokens = KamprTokens(SoftTheme, fonts, typography(fonts, SoftTheme.label, TypeScale.Phone))
    setContent {
        CompositionLocalProvider(
            LocalTokens provides tokens,
            LocalSafeArea provides SafeArea.None,
            LocalPaneChrome provides PaneChrome(chromeTop),
        ) {
            Box(Modifier.size(DESK.first, DESK.second)) {
                Box(Modifier.fillMaxSize()) {
                    FileSheet(
                        path = PATH,
                        state = Peeked.Words(BODY),
                        onClose = {},
                        onCopy = {},
                        chromeTop = chromeTop,
                        chromeBottom = 0.dp,
                    )
                }
            }
        }
    }
    waitForIdle()
}

// The operator's report, on this platform: "gives me a screen with no copy paste/anything, i also
// need to press escape to close no close button". The button was always drawn — under a bar the
// browser was painting on top of it. Nothing about the safe area was involved: it is zero here.
@OptIn(ExperimentalTestApi::class)
class BrowserFileSheetTest {
    @Test
    fun theCloseButtonClearsThePaneHeaderPaintedOverThisSurface() = runComposeUiTest {
        sheet(DESK_CHROME)
        val close = onNodeWithContentDescription("Close $PATH").getUnclippedBoundsInRoot()
        assertTrue(
            close.top >= DESK_CHROME,
            "the close button sits at ${close.top}, under $DESK_CHROME of pane header",
        )
        assertTrue(close.right <= DESK.first, "the close button is off the right edge at ${close.right}")
    }

    @Test
    fun theWholeFileHasACopyOfItsOwnBesideTheTitle() = runComposeUiTest {
        sheet(DESK_CHROME)
        val copy = onNodeWithContentDescription("Copy $PATH").getUnclippedBoundsInRoot()
        assertTrue(copy.top >= DESK_CHROME, "the copy button sits at ${copy.top}, under the header")
        onNodeWithText(BODY, substring = true, useUnmergedTree = true).assertExists()
    }
}
