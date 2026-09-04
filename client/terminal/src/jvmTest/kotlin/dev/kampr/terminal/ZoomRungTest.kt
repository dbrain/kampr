package dev.kampr.terminal

import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.runComposeUiTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.sheet(): PaneSession {
    val session = PaneSession(Phone.PANE)
    phoneTerminal(Phone.shell(rows = 40, caretRow = 6), session, io = ReadableIo)
    session.view.sheetOpen = true
    waitForIdle()
    assertTrue(session.view.zoom > 0f, "the pane never took a zoom, so nothing is tested")
    return session
}

// The rung the ladder did not have. Every control on this sheet multiplies — the slider, the wheel,
// the pinch, the +/- keys — and the three presets were fit-width, 1.2x and 1.7x, so a pane opened
// at 1.0x could be left and never returned to exactly. The operator's report: *"the config / zoom
// options really hate 1.0x"*.
@OptIn(ExperimentalTestApi::class)
class ZoomRungTest {
    @Test
    fun theSheetOffersTheCellAtItsOwnSize() = runComposeUiTest {
        val session = sheet()
        assertTrue(session.view.zoom != 1f, "the pane opened at 1.0x, so the press proves nothing")
        onNodeWithContentDescription("Actual", substring = true).performScrollTo().performClick()
        waitForIdle()
        assertEquals(1f, session.view.zoom, 0.001f, "Actual left the zoom at ${session.view.zoom}x")
    }

    // The keys are the same four rungs in the same order, so a desk never has to reach for the row.
    @Test
    fun theNumberKeysWalkTheSameFourRungs() = runComposeUiTest {
        val session = sheet()
        onRoot().performKeyInput { pressKey(Key.Two) }
        waitForIdle()
        assertEquals(1f, session.view.zoom, 0.001f, "key 2 left the zoom at ${session.view.zoom}x")

        onRoot().performKeyInput { pressKey(Key.Three) }
        waitForIdle()
        assertTrue(session.view.zoom > 1.1f, "key 3 is not Readable: ${session.view.zoom}x")

        onRoot().performKeyInput { pressKey(Key.Four) }
        waitForIdle()
        assertTrue(session.view.zoom > 1.6f, "key 4 is not Close up: ${session.view.zoom}x")
    }
}
