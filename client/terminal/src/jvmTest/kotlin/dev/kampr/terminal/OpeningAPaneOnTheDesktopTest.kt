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

    // **The other end of the same answer.** A small pane used to be magnified until it filled the
    // screen, which on a phone took a 40x12 one past 1.5x and on a desk to 3.6x — the operator's
    // *"other times 3.7x which is ridiculously large"*. Blank space beside a small pane is what a
    // small pane looks like; a pane drawn at three times its size to avoid it is not the same pane.
    @Test
    fun aSmallPaneIsNotMagnifiedToFillAPhoneEither() = runComposeUiTest {
        val session = opened(grid(cols = 40, rows = 12), 411.dp, 914.dp)
        assertEquals(
            1f,
            session.view.zoom,
            0.001f,
            "a phone magnified a 40x12 pane to ${session.view.zoom}x to fill itself",
        )
    }

    // **And a floor under it, for the same reason the ceiling is over it.** Fitting a wide pane to
    // the window is the right answer only while the result can still be read: a 300-column pane in
    // a desk window fitted to **0.7x**, and the operator's own words for what that is worth were
    // "default is often 0.4x and is tiny tiny … maybe we push towards 1.0x being at least default".
    // The whole pane at a size nobody can read is not a view of it, and Fit width is one tap and
    // one keystroke away for the times it is what you want.
    @Test
    fun aWidePaneOnADesktopOpensAtASizeThatCanBeRead() = runComposeUiTest {
        val session = opened(grid(cols = 300, rows = 60), DESKTOP_WIDTH, DESKTOP_HEIGHT)
        assertEquals(
            1f,
            session.view.zoom,
            0.001f,
            "a 300-column pane opened at ${session.view.zoom}x, which is 13sp of text scaled by that",
        )
    }

    // The floor is not the breakpoint's, it is the window's own arithmetic: it holds wherever 1.0x
    // still leaves a usable pane's worth of columns on the screen, so a split half and a rotated
    // phone get it as well as a desk. This is that window — narrower than a desk, wider than 80
    // columns of 13sp text.
    @Test
    fun aSplitSizedViewGetsTheSameFloor() = runComposeUiTest {
        val session = opened(grid(cols = 300, rows = 60), 800.dp, 900.dp)
        assertTrue(
            session.view.zoom >= 1f,
            "a half-window view opened a 300-column pane at ${session.view.zoom}x",
        )
    }

    // **And a phone holds 1.0x too, which is the operator's own answer to the fit.** This used to
    // shrink a 300-column pane until all of it was on the screen, and what that produced was
    // *"sometimes being zoomed at 0.4x which is ridiculously small"* — a fifth of a pane at a size
    // nobody can read is not a view of it either. Panning and the zoom sheet's fit-width are one
    // gesture away; a size nobody can read is not.
    @Test
    fun aPhoneOpensAPaneTooWideForItAtOneToOneRatherThanShrinkingItToNothing() = runComposeUiTest {
        val session = opened(grid(cols = 300, rows = 200), 411.dp, 914.dp)
        assertEquals(
            1f,
            session.view.zoom,
            0.001f,
            "a 300x200 pane opened at ${session.view.zoom}x on a phone",
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
