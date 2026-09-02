package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.platform.ClipboardManager
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.rightClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.withKeyDown
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import dev.kampr.shared.platform.LocalClipboardText
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.PaneChrome
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.paneActions
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.render.GridPoint
import dev.kampr.terminal.render.Selection
import dev.kampr.terminal.view.TerminalView
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val COPY = "Copy the selection to the clipboard"
private const val PASTE = "Paste the clipboard into this pane"
private const val ALL = "Select the whole grid"

private class Watching(override val readOnly: Boolean = false) : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }
    override fun prefs(paneId: String) = PanePrefs()

    val typed: String get() = sent.filterIsInstance<ClientMsg.InputText>().joinToString("") { it.text }
}

@Suppress("DEPRECATION")
private class Slate : ClipboardManager {
    var held: AnnotatedString? = null
    override fun setText(annotatedString: AnnotatedString) {
        held = annotatedString
    }
    override fun getText(): AnnotatedString? = held
    override fun hasText(): Boolean = held != null
}

// A mosaic cell, as far as this gesture is concerned: `MosaicCell` puts `Modifier.paneActions` on
// the cell and that runs on the Initial pass and consumes. The mosaic module depends on this one
// rather than the other way round, so the *composition* is reproduced here rather than imported.
private class Managing : ManageIo {
    var opened: String? = null
    override val enabled: Boolean get() = true
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) {
        opened = paneId
    }
}

@OptIn(ExperimentalTestApi::class)
private class Rig(val io: Watching, val board: Slate, val session: PaneSession, val manage: Managing)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.terminal(
    io: Watching = Watching(),
    clipboard: suspend () -> String? = { "cargo test" },
    inACell: Boolean = false,
): Rig {
    val board = Slate()
    val session = PaneSession(Phone.PANE)
    val manage = Managing()
    setContent {
        CompositionLocalProvider(
            LocalTokens provides Phone.tokens(),
            LocalPaneIo provides io,
            LocalPaneChrome provides PaneChrome(Phone.HEADER),
            LocalClipboardManager provides board,
            LocalClipboardText provides clipboard,
            LocalManage provides manage,
        ) {
            Box(Modifier.size(411.dp, 914.dp)) {
                Box(
                    Modifier
                        .fillMaxSize()
                        .then(if (inACell) Modifier.paneActions(Phone.PANE) else Modifier),
                ) {
                    TerminalView(Phone.shell(), session, io)
                }
            }
        }
    }
    waitForIdle()
    session.openKeyboard()
    waitForIdle()
    return Rig(io, board, session, manage)
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.chord(down: List<Key>, key: Key) {
    onNode(hasSetTextAction()).performKeyInput {
        fun press(remaining: List<Key>) {
            if (remaining.isEmpty()) pressKey(key) else withKeyDown(remaining.first()) { press(remaining.drop(1)) }
        }
        press(down)
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.rightClickTheGrid() {
    onNodeWithContentDescription("Terminal grid", substring = true).performMouseInput {
        rightClick(Offset(width * 0.4f, height * 0.5f))
    }
    waitForIdle()
}

private fun select(session: PaneSession) {
    session.view.selection = Selection(GridPoint(0, 0), GridPoint(0, 10))
}

// Two reports in one surface. The copy chord was swallowed: `ctrl+shift+C` — what every Linux
// terminal emulator copies with — arrived as `e.key === "C"`, was lowercased and sent to the pane
// as `^C`, so **copying interrupted the process**, and on macOS `⌘C` did the same. And a
// right-click over the grid did nothing at all: `Modifier.paneActions` is on the sidebar cards, the
// herd list and the mosaic cells, and never on the one surface a desk right-clicks — while
// `boot.js` had already taken the browser's own menu away over the canvas.
@OptIn(ExperimentalTestApi::class)
class CopyingFromAPaneTest {
    @Test
    fun theCopyChordCopiesTheSelectionInsteadOfInterruptingThePane() = runComposeUiTest {
        val rig = terminal()
        select(rig.session)
        chord(listOf(Key.CtrlLeft, Key.ShiftLeft), Key.C)
        assertEquals(
            "[20:36:31 d",
            rig.board.held?.text,
            "ctrl+shift+C put nothing on the clipboard",
        )
        assertEquals("", rig.io.typed, "ctrl+shift+C sent the pane a control code")
        assertNull(rig.session.view.selection, "the selection outlived the copy")
    }

    @Test
    fun theCommandChordCopiesAndIsNeverAControlCode() = runComposeUiTest {
        val rig = terminal()
        select(rig.session)
        chord(listOf(Key.MetaLeft), Key.C)
        assertEquals("[20:36:31 d", rig.board.held?.text, "⌘C put nothing on the clipboard")
        assertEquals("", rig.io.typed, "Command-C was sent to the pane as SIGINT")
    }

    // The half that must not change. A terminal without ctrl+C is not a terminal.
    @Test
    fun plainCtrlCStillInterruptsThePane() = runComposeUiTest {
        val rig = terminal()
        select(rig.session)
        chord(listOf(Key.CtrlLeft), Key.C)
        assertEquals("\u0003", rig.io.typed, "ctrl+C stopped interrupting the pane")
        assertNull(rig.board.held, "ctrl+C copied the selection instead of interrupting")
    }

    // And every other shifted control letter, which is standard terminal behaviour: only C and V
    // are taken off the pane.
    @Test
    fun anotherShiftedControlLetterStillReachesThePane() = runComposeUiTest {
        val rig = terminal()
        chord(listOf(Key.CtrlLeft, Key.ShiftLeft), Key.A)
        assertEquals("\u0001", rig.io.typed, "ctrl+shift+A stopped producing its control byte")
    }

    @Test
    fun theCopyChordWithNothingSelectedDoesNothingRatherThanSendingAStrayByte() = runComposeUiTest {
        val rig = terminal()
        chord(listOf(Key.CtrlLeft, Key.ShiftLeft), Key.C)
        assertEquals("", rig.io.typed, "a copy with no selection went to the pane as a byte")
        assertNull(rig.board.held, "a copy with no selection put something on the clipboard")
    }

    @Test
    fun thePasteChordsPutTheClipboardIntoThePane() = runComposeUiTest {
        for (down in listOf(listOf(Key.CtrlLeft, Key.ShiftLeft), listOf(Key.MetaLeft))) {
            runComposeUiTest {
                val rig = terminal()
                chord(down, Key.V)
                waitForIdle()
                assertEquals(
                    "[200~cargo test[201~",
                    rig.io.typed,
                    "$down+V did not paste the clipboard, bracketed (#9)",
                )
            }
        }
    }

    @Test
    fun aRightClickOverTheGridOpensAMenuAtThePointer() = runComposeUiTest {
        val rig = terminal()
        select(rig.session)
        rightClickTheGrid()
        assertTrue(rig.session.view.menuAt != null, "the right-click opened nothing")
        onNodeWithContentDescription(COPY).assertExists()
        onNodeWithContentDescription(PASTE).assertExists()
        onNodeWithContentDescription(ALL).assertExists()
    }

    @Test
    fun theMenuOffersNoCopyWithNothingSelected() = runComposeUiTest {
        val rig = terminal()
        rightClickTheGrid()
        assertTrue(rig.session.view.menuAt != null, "the right-click opened nothing")
        assertTrue(
            onAllNodesWithContentDescription(COPY).fetchSemanticsNodes().isEmpty(),
            "a Copy that can copy nothing was offered anyway",
        )
        onNodeWithContentDescription(ALL).assertExists()
    }

    // Absent rather than present-and-refusing, which is what the selection pill already does with
    // its own Paste and what `ManageLayer` does with everything a write can reach.
    @Test
    fun aReadOnlyDeviceIsOfferedNoPaste() = runComposeUiTest {
        terminal(io = Watching(readOnly = true))
        rightClickTheGrid()
        assertTrue(
            onAllNodesWithContentDescription(PASTE).fetchSemanticsNodes().isEmpty(),
            "a device that may not type was offered a paste",
        )
        onNodeWithContentDescription(ALL).assertExists()
    }

    @Test
    fun eachMenuItemDoesWhatItSays() = runComposeUiTest {
        val rig = terminal()
        select(rig.session)
        rightClickTheGrid()
        onNodeWithContentDescription(COPY).performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        assertEquals("[20:36:31 d", rig.board.held?.text, "the menu's Copy copied nothing")
        assertNull(rig.session.view.menuAt, "the menu stayed up after a press")

        rightClickTheGrid()
        onNodeWithContentDescription(ALL).performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        val whole = rig.session.view.selection
        assertEquals(GridPoint(0, 0), whole?.start, "Select all did not start at the first cell")
        assertEquals(
            GridPoint(39, 93),
            whole?.end,
            "Select all did not reach the last cell of the grid",
        )

        rig.session.view.menuAt = null
        waitForIdle()
        rightClickTheGrid()
        onNodeWithContentDescription(PASTE).performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        assertEquals(
            "[200~cargo test[201~",
            rig.io.typed,
            "the menu's Paste sent the pane nothing",
        )
    }

    // Inside a mosaic the cell's own sheet wins, because `paneActions` runs on the Initial pass and
    // consumes the press. The grid's handler asks for an unconsumed one, which is the whole of the
    // arbitration between them.
    @Test
    fun aRightClickInsideAMosaicCellOpensTheCellsSheetAndNotTheGridsMenu() = runComposeUiTest {
        val rig = terminal(inACell = true)
        select(rig.session)
        rightClickTheGrid()
        assertEquals(Phone.PANE, rig.manage.opened, "the cell's actions sheet never opened")
        assertNull(rig.session.view.menuAt, "the grid's menu opened inside a mosaic cell")
    }

    // And on the pane screen, where nothing above the grid claims the gesture, the opposite: the
    // menu opens and no actions sheet is asked for.
    @Test
    fun aRightClickOnThePaneScreenOpensNoActionsSheet() = runComposeUiTest {
        val rig = terminal()
        rightClickTheGrid()
        assertNull(rig.manage.opened, "the pane screen opened an actions sheet on a right-click")
        assertTrue(rig.session.view.menuAt != null, "the grid's own menu never opened")
    }
}
