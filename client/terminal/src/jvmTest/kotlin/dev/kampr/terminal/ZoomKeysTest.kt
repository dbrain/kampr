package dev.kampr.terminal

import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.runComposeUiTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The sheet is the one place a shortcut can be taken without a fight. While the grid is live the
// terminal owns the keyboard — `FieldTextInput` consumes every ctrl chord it recognises and drops
// the rest, and the wasm layer `preventDefault`s Escape and the chord set into the shell — so a
// global binding would either be swallowed or steal a key the shell needs. Scoped to an open
// sheet, nothing is competing for them.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.openSheet(): PaneSession {
    val session = PaneSession(Phone.PANE)
    phoneTerminal(Phone.shell(rows = 40, caretRow = 6), session)
    session.view.sheetOpen = true
    waitForIdle()
    assertTrue(session.view.zoom > 0f, "the pane has to have taken a zoom, or nothing is tested")
    return session
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.press(key: Key) {
    onRoot().performKeyInput { pressKey(key) }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
class ZoomKeysTest {
    @Test
    fun plusZoomsIn() = runComposeUiTest {
        val session = openSheet()
        val before = session.view.zoom
        press(Key.Equals)
        assertTrue(session.view.zoom > before, "the sheet left the zoom at $before")
    }

    @Test
    fun minusZoomsOut() = runComposeUiTest {
        val session = openSheet()
        val before = session.view.zoom
        press(Key.Minus)
        assertTrue(session.view.zoom < before, "the sheet left the zoom at $before")
    }

    // Arrows are the fine adjustment, which is the whole reason a keyboard is worth having here:
    // a preset is a jump and the wheel is a click, and neither lands on a particular size.
    @Test
    fun theArrowsAreAFinerStepThanThePlusKey() = runComposeUiTest {
        val session = openSheet()
        val floor = session.view.zoom
        press(Key.Equals)
        val coarse = session.view.zoom / floor

        val other = openSheet()
        val base = other.view.zoom
        press(Key.DirectionUp)
        val fine = other.view.zoom / base
        assertTrue(fine > 1f, "the up arrow did not move the zoom at all")
        assertTrue(fine < coarse, "the arrow moved $fine where the plus key moved $coarse")
    }

    // The three digits are the three presets in the row's own order, so 3 is further in than 2 and
    // landing on one is idempotent — a preset is a place, not a step.
    @Test
    fun theDigitsPickThePresets() = runComposeUiTest {
        val session = openSheet()
        press(Key.Two)
        val readable = session.view.zoom
        press(Key.Three)
        val closeUp = session.view.zoom
        assertTrue(closeUp > readable, "3 landed at $closeUp, no further in than 2 at $readable")
        press(Key.Two)
        assertEquals(readable, session.view.zoom, 0.001f, "going back to 2 did not land where 2 was")
    }

    // A key that means nothing to the sheet must not be eaten, or Escape stops closing it.
    @Test
    fun theSheetStillClosesOnEscape() = runComposeUiTest {
        val session = openSheet()
        press(Key.Escape)
        assertTrue(!session.view.sheetOpen, "the sheet swallowed Escape instead of closing")
    }
}
