package dev.kampr.terminal

import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val DESKTOP_WIDTH = 1624.dp
private val DESKTOP_HEIGHT = 1000.dp

private class RecordingIo : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }

    override fun prefs(paneId: String) = PanePrefs()
}

private fun grid(cols: Int, rows: Int): PaneState {
    val pane = PaneState(Phone.PANE, StyleTable())
    val line = "$ ls"
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = cols,
            rows = rows,
            rowsData = listOf(RowDiff(0, listOf(Run(0, line)))),
            cursor = Cursor(line.length, 0, true),
            links = emptyList(),
        ),
    )
    return pane
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.opened(
    pane: PaneState,
    width: Dp,
    height: Dp,
    io: PaneIo = HushIo,
): PaneSession {
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width, height, io)
    assertTrue(session.view.zoom > 0f, "the pane never adopted a zoom, so nothing is tested")
    return session
}

// A fresh herdr pane is narrow — 80x24, often less — and a desktop window is not. `defaultZoom`
// fills at least one axis, which is right on a phone where the screen is the constraint, and on a
// desktop blows a 40x12 pane up to 3.5x of a size that was already legible.
@OptIn(ExperimentalTestApi::class)
class OpeningAPaneOnTheDesktopTest {
    @Test
    fun aFreshNarrowPaneOnADesktopDoesNotOpenMagnified() = runComposeUiTest {
        val session = opened(grid(cols = 40, rows = 12), DESKTOP_WIDTH, DESKTOP_HEIGHT)
        assertEquals(
            1f,
            session.view.zoom,
            0.001f,
            "a fresh 40x12 pane opened at ${session.view.zoom}x in a desktop window",
        )
    }

    // The ceiling is on the breakpoint, not on the zoom. A phone has no room to spare: capping it
    // there letterboxes a 24-row pane, which is exactly what `max`-not-`min` in SurfaceGeometry
    // exists to prevent. This test is what stops the fix being simplified into a global cap.
    @Test
    fun theSameNarrowPaneOnAPhoneStillFillsTheScreen() = runComposeUiTest {
        val session = opened(grid(cols = 40, rows = 12), 411.dp, 914.dp)
        assertTrue(
            session.view.zoom > 1.5f,
            "a phone letterboxed a 40x12 pane at ${session.view.zoom}x instead of filling the screen",
        )
    }

    // A ceiling, not a pin: a grid the window cannot hold still shrinks until it does.
    @Test
    fun aGridTooBigForTheDesktopWindowStillShrinksToFit() = runComposeUiTest {
        val session = opened(grid(cols = 300, rows = 60), 1000.dp, 700.dp)
        assertTrue(
            session.view.zoom < 0.9f,
            "a 300x60 grid held its size at ${session.view.zoom}x in a 1000x700 window",
        )
    }

    // An operator who deliberately chose 3x keeps 3x. The ceiling governs the computed default and
    // nothing else.
    @Test
    fun aStoredZoomStillWinsOverTheDesktopCeiling() = runComposeUiTest {
        val session = opened(grid(cols = 40, rows = 12), DESKTOP_WIDTH, DESKTOP_HEIGHT, ReadableIo)
        assertEquals(
            1.2f,
            session.view.zoom,
            0.001f,
            "the ceiling overrode a zoom the operator had chosen, landing at ${session.view.zoom}x",
        )
    }

    // Only a chosen zoom is persisted, so there are no stale computed prefs for the ceiling to
    // have to migrate — and the ceiling must not start writing one either.
    @Test
    fun theComputedDefaultIsStillNotWrittenBackToPrefs() = runComposeUiTest {
        val io = RecordingIo()
        val session = opened(grid(cols = 40, rows = 12), DESKTOP_WIDTH, DESKTOP_HEIGHT, io)
        mainClock.advanceTimeBy(2_000)
        waitForIdle()
        assertTrue(
            io.sent.none { it is ClientMsg.SetPrefs },
            "the computed default was written back to prefs: ${io.sent.filterIsInstance<ClientMsg.SetPrefs>()}",
        )

        session.view.sheetOpen = true
        waitForIdle()
        onRoot().performKeyInput { pressKey(Key.Equals) }
        waitForIdle()
        assertTrue(session.view.chosen, "the sheet did not take the key, so the harness proves nothing")
        mainClock.advanceTimeBy(2_000)
        waitForIdle()
        assertTrue(
            io.sent.filterIsInstance<ClientMsg.SetPrefs>().isNotEmpty(),
            "no pref was written even for a chosen zoom, so the assertion above proves nothing",
        )
    }
}
