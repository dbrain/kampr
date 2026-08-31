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
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.platform.LocalClipboardText
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.view.TerminalView
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

private val WIDTH = 411.dp

private const val PASTE_START = "\u001b[200~"
private const val PASTE_END = "\u001b[201~"

private const val PASTE = "Paste the clipboard into the pane"
private const val COPY = "Copy the selection"
private const val BLOCK = "Select by column instead of by line"

private class Recording(override val readOnly: Boolean = false) : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }
    override fun prefs(paneId: String) = PanePrefs()
}

// `InputSink.paste` — the bracketing, and the guard's inspection of a paste that carries its own
// Enter — was written, tested, and reachable from nothing: no surface in the app called it. On a
// phone the terminal's context menu is the pill a long press raises, and it offered Copy and a
// selection mode and no way at all to put text into the pane.
@OptIn(ExperimentalTestApi::class)
class PastingIntoAPaneTest {
    private fun ComposeUiTest.terminal(io: PaneIo, clipboard: suspend () -> String?): PaneSession {
        val session = PaneSession(Phone.PANE)
        setContent {
            CompositionLocalProvider(
                LocalTokens provides Phone.tokens(),
                LocalPaneIo provides io,
                LocalClipboardText provides clipboard,
            ) {
                Box(Modifier.size(WIDTH, 914.dp)) {
                    Box(Modifier.fillMaxSize()) { TerminalView(Phone.shell(), session, io) }
                }
            }
        }
        waitForIdle()
        return session
    }

    // The gesture from the report, not a selection set by hand: a press that stays still is what
    // raises the pill, and the pill is the only place a phone can ask for a paste.
    private fun ComposeUiTest.longPressTheGrid(across: Float = 0.5f) {
        onNodeWithContentDescription("Terminal grid", substring = true).performTouchInput {
            val at = Offset(width * across, center.y)
            down(at)
            advanceEventTime(900)
            moveTo(at)
            up()
        }
        waitForIdle()
    }

    @Test
    fun aLongPressOffersPasteAndTheClipboardArrivesBracketed() = runComposeUiTest {
        val io = Recording()
        val session = terminal(io) { "cargo test -p kampr-term" }
        longPressTheGrid()
        assertNotNull(session.view.selection, "the long press raised no pill at all")

        onNodeWithContentDescription(PASTE).performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()

        assertEquals(
            listOf(ClientMsg.InputText(Phone.PANE, PASTE_START + "cargo test -p kampr-term" + PASTE_END)),
            io.sent.filterIsInstance<ClientMsg.InputText>(),
            "probe #9: pane.send_text frames nothing itself, so an unbracketed multi-line paste " +
                "runs line by line in a shell",
        )
    }

    // Pressing it takes the pill away, because the read is what raises Android's own "pasted from
    // your clipboard" notice and a pill still sitting under that notice reads as a press that did
    // nothing.
    @Test
    fun pressingPasteClosesThePill() = runComposeUiTest {
        val io = Recording()
        val session = terminal(io) { "ls" }
        longPressTheGrid()
        onNodeWithContentDescription(PASTE).performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        assertEquals(null, session.view.selection)
    }

    // A clipboard with nothing on it, and a browser that refused the read, look the same from here.
    // Either way the operator pressed a thing and has to be told why nothing arrived.
    @Test
    fun anEmptyClipboardIsSaidRatherThanSilentlyDoingNothing() = runComposeUiTest {
        val io = Recording()
        terminal(io) { null }
        longPressTheGrid()
        onNodeWithContentDescription(PASTE).performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()

        assertTrue(io.sent.isEmpty(), "an empty clipboard was sent to the pane anyway")
        onNodeWithContentDescription("Nothing on the clipboard to paste.", substring = true).assertExists()
    }

    // The report: "the paste from terminal when I long press on side of screen draws off screen".
    // The pill is placed at the selection's own start column and was clamped at the left margin
    // and nowhere else, so a press in the right-hand third put Copy on the screen and hung Paste
    // and the mode toggle past its edge — the one affordance a long press exists to offer, and it
    // is unreachable exactly where a right-handed thumb lands.
    @Test
    fun aPillRaisedAtTheEdgeOfTheScreenIsStillOnTheScreen() = runComposeUiTest {
        val io = Recording()
        terminal(io) { "ls" }
        longPressTheGrid(across = 0.97f)

        val pill = onNodeWithContentDescription(BLOCK).getUnclippedBoundsInRoot()
        assertTrue(pill.right <= WIDTH, "the pill's last button ends at ${pill.right}, past the ${WIDTH} screen")
        assertTrue(pill.left >= 0.dp, "and the pill was pushed off the other edge instead")
    }

    @Test
    fun aPillRaisedAtTheFirstColumnKeepsItsLeftEdge() = runComposeUiTest {
        val io = Recording()
        terminal(io) { "ls" }
        longPressTheGrid(across = 0.01f)
        assertTrue(onNodeWithContentDescription(COPY).getUnclippedBoundsInRoot().left >= 0.dp)
    }

    // Absent, not present-and-refusing, like every other write affordance on this surface.
    @Test
    fun aReadOnlyDeviceIsOfferedNoPasteAtAll() = runComposeUiTest {
        val io = Recording(readOnly = true)
        terminal(io) { "rm -rf /" }
        longPressTheGrid()

        onNodeWithContentDescription(COPY).assertExists()
        assertEquals(
            0,
            onAllNodesWithContentDescription(PASTE).fetchSemanticsNodes().size,
            "a read-only device was offered a paste it may not make",
        )
    }
}
