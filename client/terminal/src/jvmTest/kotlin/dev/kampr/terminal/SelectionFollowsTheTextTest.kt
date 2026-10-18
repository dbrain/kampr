package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneChrome
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.render.GridPoint
import dev.kampr.terminal.render.Selection
import dev.kampr.terminal.view.TerminalView
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull

private const val ROWS = 40

private fun screen(pane: PaneState, scrolled: Int) {
    pane.applyReset(
        ServerMsg.GridReset(
            pane = pane.id,
            cols = 94,
            rows = ROWS,
            rowsData = (0 until ROWS).map { RowDiff(it, listOf(Run(0, "● reply line ${it + scrolled}"))) },
            cursor = Cursor(0, ROWS - 1, false),
            links = emptyList(),
        ),
    )
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.terminal(pane: PaneState, session: PaneSession): Slate {
    val board = Slate()
    setContent {
        CompositionLocalProvider(
            LocalTokens provides Phone.tokens(),
            LocalPaneIo provides HushIo,
            LocalPaneChrome provides PaneChrome(Phone.HEADER),
            LocalClipboardManager provides board,
        ) {
            Box(Modifier.size(411.dp, 914.dp)) { TerminalView(pane, session, HushIo) }
        }
    }
    waitForIdle()
    return board
}

// The operator, on the desk: *"when i select text and the chat moves the selection moves with it,
// can it stay locked on the text?"* — a full-screen program scrolls its own text through the same
// cells, and a selection held as cell indices stayed on the cells.
@OptIn(ExperimentalTestApi::class)
class SelectionFollowsTheTextTest {
    @Test
    fun aSelectionStaysOnTheTextAsTheProgramScrollsIt() = runComposeUiTest {
        val pane = PaneState(Phone.PANE, StyleTable())
        screen(pane, scrolled = 0)
        val session = PaneSession(Phone.PANE)
        terminal(pane, session)
        session.view.selection = Selection(GridPoint(20, 2), GridPoint(21, 12))
        waitForIdle()

        screen(pane, scrolled = 3)
        waitForIdle()

        assertEquals(
            Selection(GridPoint(17, 2), GridPoint(18, 12)),
            session.view.selection,
            "the selection stayed on the cells while its text scrolled away from under it",
        )
    }

    @Test
    fun aSelectionWhoseTextHasGoneStaysWhereItIs() = runComposeUiTest {
        val pane = PaneState(Phone.PANE, StyleTable())
        screen(pane, scrolled = 0)
        val session = PaneSession(Phone.PANE)
        terminal(pane, session)
        val held = Selection(GridPoint(20, 2), GridPoint(21, 12))
        session.view.selection = held
        waitForIdle()

        screen(pane, scrolled = 500)
        waitForIdle()

        assertEquals(held, session.view.selection, "a selection with nothing to follow moved anyway")
    }

    // The desk convention: what the mouse selects is on the clipboard when the button comes up.
    @Test
    fun aMouseSelectionIsCopiedWhenTheButtonComesUp() = runComposeUiTest {
        val pane = PaneState(Phone.PANE, StyleTable())
        screen(pane, scrolled = 0)
        val session = PaneSession(Phone.PANE)
        val board = terminal(pane, session)
        onNodeWithContentDescription("Terminal grid", substring = true).performMouseInput {
            val y = center.y
            moveTo(Offset(60f, y))
            press()
            moveTo(Offset(180f, y))
            moveTo(Offset(300f, y))
            release()
        }
        waitForIdle()

        val selection = assertNotNull(session.view.selection, "the drag selected nothing")
        val copied = assertNotNull(board.held, "the mouse selection was not copied")
        assertEquals(true, copied.text.isNotEmpty(), "an empty copy for a selection of $selection")
        assertNotNull(session.view.selection, "copying took the selection off the screen")
    }

    // A finger has a pill with Copy on it and Android announces every clipboard write, so a long
    // press is left to ask.
    @Test
    fun aLongPressSelectionIsNotCopiedUntilAsked() = runComposeUiTest {
        val pane = PaneState(Phone.PANE, StyleTable())
        screen(pane, scrolled = 0)
        val session = PaneSession(Phone.PANE)
        val board = terminal(pane, session)
        onNodeWithContentDescription("Terminal grid", substring = true).performTouchInput {
            down(center)
            advanceEventTime(1_000)
            up()
        }
        waitForIdle()

        assertNotNull(session.view.selection, "the long press selected nothing")
        assertNull(board.held, "a long press copied without being asked")
    }
}
