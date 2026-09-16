package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.platform.LocalClipboardText
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.TerminalView
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// pi's `/model` and `/thinking`, measured on the desk (research/probe/pi-selector.py): the
// selector replaces the composer, its search line takes the caret, and the options sit *below*
// it, the last of them on the grid's last row. The operator's report for the phone reading of
// that screen: "I cannot see the options at all, it locks the bottom of the screen to the text
// entry line" — the surface had rested where the composer sat at the bottom of the screen, the
// band, which keeps the caret on screen, let it stay there, and the list below the caret was out
// of reach.
@OptIn(ExperimentalTestApi::class)
class ContentBelowTheCaretTest {
    private class Recording : PaneIo {
        val sent = mutableListOf<ClientMsg>()
        override fun send(msg: ClientMsg) {
            sent += msg
        }
        override fun prefs(paneId: String) = PanePrefs()
    }

    // A conversation with the composer and its two footer rows, the caret on the composer, and
    // nothing written below it — the shape of an idle pi pane, tall enough to overflow a phone.
    private fun idle(caretRow: Int, total: Int): PaneState {
        val pane = PaneState(Phone.PANE, StyleTable())
        val lines = (0..caretRow).map { "conversation row $it" } +
            listOf("", "composer", "footer left", "footer right")
        pane.applyReset(
            ServerMsg.GridReset(
                pane = Phone.PANE,
                cols = 94,
                rows = total,
                rowsData = lines.mapIndexed { index, text ->
                    RowDiff(index, text.ifBlank { null }?.let { listOf(Run(0, it)) } ?: emptyList())
                },
                cursor = Cursor(0, caretRow, true),
                links = emptyList(),
            ),
        )
        return pane
    }

    // The same pane with the selector open: the caret on the search line above the composer's
    // old row, and the options filling every row down to the last of the grid.
    private fun selector(pane: PaneState, searchRow: Int, firstOption: Int, lastRow: Int) {
        val options = (firstOption..lastRow).map { RowDiff(it, listOf(Run(0, "option $it"))) }
        pane.applyPatch(
            ServerMsg.GridPatch(
                pane = Phone.PANE,
                rows = options,
                cursor = Cursor(1, searchRow, true),
                links = emptyList(),
            ),
        )
    }

    private fun ComposeUiTest.terminal(pane: PaneState, io: Recording, clipboard: String): PaneSession {
        val session = PaneSession(Phone.PANE)
        setContent {
            CompositionLocalProvider(
                LocalTokens provides Phone.tokens(),
                LocalPaneIo provides io,
                LocalClipboardText provides { clipboard },
            ) {
                Box(Modifier.size(411.dp, 914.dp)) {
                    Box(Modifier.fillMaxSize()) { TerminalView(pane, session, io) }
                }
            }
        }
        waitForIdle()
        return session
    }

    // The operator's byte, the way a phone sends one: the long-press pill and the clipboard.
    private fun ComposeUiTest.paste() {
        onNodeWithContentDescription("Terminal grid", substring = true).performTouchInput {
            val at = Offset(width * 0.5f, center.y)
            down(at)
            advanceEventTime(900)
            moveTo(at)
            up()
        }
        waitForIdle()
        onNodeWithContentDescription("Paste the clipboard into the pane")
            .performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
    }

    @Test
    fun aListTheOperatorOpenedBelowTheCaretBringsTheBottomOfTheListToTheBottomOfTheScreen() {
        runComposeUiTest {
            val total = 200
            val io = Recording()
            val pane = idle(caretRow = 170, total = total)
            val session = terminal(pane, io, "/model\r")
            assertTrue(session.view.maxScroll > 0f, "the grid has to overflow, or nothing is tested")
            val resting = session.view.scrollY
            assertTrue(resting > 0f, "the surface had to rest above the end of the grid first")

            paste()
            assertTrue(io.sent.isNotEmpty(), "the operator's byte never reached the pane")

            selector(pane, searchRow = 171, firstOption = 174, lastRow = total - 1)
            waitForIdle()

            // The last option is content on the grid's last row, so the end of the record is the
            // end of the grid: a rest that hides it is a rest that hides the list.
            assertEquals(
                0f,
                session.view.scrollY,
                "the options below the search line are out of reach: the surface is at " +
                    "${session.view.scrollY} of ${session.view.maxScroll}",
            )
        }
    }

    // The pane repainting the same shape on its own — no byte sent — owes the operator nothing:
    // the surface rests where it rested, which is the whole of the sweep rule in this shape.
    @Test
    fun aListThePaneOpensOnItsOwnDoesNotMoveTheSurface() {
        runComposeUiTest {
            val total = 200
            val io = Recording()
            val pane = idle(caretRow = 170, total = total)
            val session = terminal(pane, io, "/model\r")
            val resting = session.view.scrollY
            assertTrue(resting > 0f, "the surface had to rest above the end of the grid first")

            selector(pane, searchRow = 171, firstOption = 174, lastRow = total - 1)
            waitForIdle()

            assertEquals(
                resting,
                session.view.scrollY,
                "a pane that repaints by itself sends nothing, and the surface moved " +
                    "$resting -> ${session.view.scrollY} for no operator's ask",
            )
        }
    }
}