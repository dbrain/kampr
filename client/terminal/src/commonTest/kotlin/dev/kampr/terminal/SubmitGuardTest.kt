package dev.kampr.terminal

import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.guard.SubmitGuard
import dev.kampr.terminal.input.Esc
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.Latches
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val PANE = "01JNODE.../w3:p1"

private class GuardIo(
    private val agent: String? = null,
    private val prefs: PanePrefs = PanePrefs(),
) : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }

    override fun prefs(paneId: String) = prefs
    override fun info(paneId: String) = PaneInfo(id = PANE, nodeId = "01JNODE", agent = agent)

    val text: List<String> get() = sent.filterIsInstance<ClientMsg.InputText>().map { it.text }
}

private fun screen(cols: Int, vararg lines: String): PaneState {
    val pane = PaneState(PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = PANE,
            cols = cols,
            rows = lines.size,
            rowsData = lines.mapIndexed { row, text -> RowDiff(row, listOf(Run(0, text))) },
            cursor = Cursor(lines.last().length, lines.size - 1, true),
            links = emptyList(),
        ),
    )
    return pane
}

private fun rig(
    pane: PaneState,
    agent: String? = null,
    prefs: PanePrefs = PanePrefs(),
): Triple<GuardIo, InputSink, SubmitGuard> {
    val io = GuardIo(agent, prefs)
    val session = PaneSession(PANE)
    val guard = SubmitGuard(pane, io, session.confirm)
    return Triple(io, InputSink(PANE, io, Latches(), guard), guard)
}

class SubmitGuardTest {
    @Test
    fun enterIsHeldOnABashPromptWithATimestamp() {
        val pane = screen(80, "[13:44 dbrain@comingclean ~/dev/kampr]\$ rm -rf build")
        val (io, sink, guard) = rig(pane)
        sink.raw(Esc.ENTER)
        assertEquals("rm -rf build", guard.state.held?.command)
        assertTrue(io.text.isEmpty(), "the Enter must not have reached the pane")
    }

    @Test
    fun enterIsHeldOnAZshPrompt() {
        val pane = screen(80, "➜  kampr git:(main) ✗ git push --force origin main")
        val (_, sink, guard) = rig(pane)
        sink.raw(Esc.ENTER)
        assertEquals("git push --force origin main", guard.state.held?.command)
    }

    // PS2 puts the loop body on its own line with no prompt of its own to strip, and it is a hard
    // line break rather than a soft wrap, so the row above must not be joined onto it.
    @Test
    fun enterIsHeldOnAContinuationLine() {
        val pane = screen(40, "\$ for f in build dist; do", ">   rm -rf \$f")
        val (_, sink, guard) = rig(pane)
        sink.raw(Esc.ENTER)
        assertEquals("rm -rf \$f", guard.state.held?.command)
    }

    // The command is longer than the grid, so the pane wrapped it across two rows. Reading only the
    // cursor's row would see `ild-cache` and pass a `rm -rf` straight through.
    @Test
    fun aSoftWrappedCommandIsJoinedBeforeItIsMatched() {
        val pane = screen(20, "\$ rm -rf /var/tmp/bu", "ild-cache")
        val (_, sink, guard) = rig(pane)
        sink.raw(Esc.ENTER)
        assertEquals("rm -rf /var/tmp/build-cache", guard.state.held?.command)
    }

    @Test
    fun onlyWhatIsLeftOfTheCursorCounts() {
        val pane = screen(40, "\$ rm -rf build")
        pane.applyPatch(ServerMsg.GridPatch(PANE, emptyList(), Cursor(4, 0, true), emptyList()))
        val (io, sink, guard) = rig(pane)
        sink.raw(Esc.ENTER)
        assertNull(guard.state.held)
        assertEquals(listOf(Esc.ENTER), io.text)
    }

    @Test
    fun anAgentPaneIsNeverGuarded() {
        val pane = screen(80, "> rm -rf build is what I would run next")
        val (io, sink, guard) = rig(pane, agent = "claude")
        sink.raw(Esc.ENTER)
        assertNull(guard.state.held)
        assertEquals(listOf(Esc.ENTER), io.text)
    }

    @Test
    fun anOrdinaryCommandGoesStraightThrough() {
        val pane = screen(80, "dbrain@comingclean ~/dev/kampr \$ cargo test -p kampr-term")
        val (io, sink, guard) = rig(pane)
        sink.raw(Esc.ENTER)
        assertNull(guard.state.held)
        assertEquals(listOf(Esc.ENTER), io.text)
    }

    @Test
    fun thePaneCanTurnItOffForItself() {
        val pane = screen(80, "\$ rm -rf build")
        val (io, sink, guard) = rig(pane, prefs = PanePrefs(mapOf("confirm" to "off")))
        sink.raw(Esc.ENTER)
        assertNull(guard.state.held)
        assertEquals(listOf(Esc.ENTER), io.text)
    }

    // A multi-line paste executes line by line in a shell that ignores the bracketing, so the
    // content is inspected before it leaves rather than after.
    @Test
    fun aPasteThatCarriesItsOwnSubmitIsInspected() {
        val pane = screen(80, "\$ ")
        val (io, sink, guard) = rig(pane)
        sink.paste("cd /tmp\nsudo rm -rf /var/lib/thing\n")
        assertNotNull(guard.state.held)
        assertTrue(io.text.isEmpty())
    }

    @Test
    fun aSingleLinePasteWaitsForTheEnterThatFollowsIt() {
        val pane = screen(80, "\$ ")
        val (io, sink, guard) = rig(pane)
        sink.paste("rm -rf build")
        assertNull(guard.state.held)
        assertEquals(1, io.text.size)
    }

    @Test
    fun typingAgainReleasesTheHold() {
        val pane = screen(80, "\$ rm -rf build")
        val (_, sink, guard) = rig(pane)
        sink.raw(Esc.ENTER)
        assertNotNull(guard.state.held)
        sink.raw(Esc.BACKSPACE)
        assertNull(guard.state.held)
    }

    // The report: alt+enter in a Claude prompt box sometimes opens a line and sometimes sends the
    // message. Alt+enter is `ESC CR`, and the only thing that tells a terminal program apart from
    // Escape-then-Enter is that the two bytes arrive in **one** `read`. Measured on a pty: written
    // together they always do; written 1 ms apart they never do. The guard cut every payload
    // carrying a submit in two whether or not it held anything, and each half crossed the wire as
    // its own `input` message — so the pane got a lone `ESC` and then a lone `CR`, a
    // `pane.send_text` round trip apart, which #273 measures at p50 1.2 ms. The chord survived
    // only when the two writes happened to land before the program was scheduled.
    //
    // So the payload is cut only when something is actually held back, which is the one case that
    // has two pieces to send at two different times.
    @Test
    fun aChordThatCarriesItsOwnSubmitCrossesTheWireInOnePiece() {
        val pane = screen(80, "\$ ")
        val (io, sink, guard) = rig(pane)
        sink.raw(Esc.ESCAPE + Esc.ENTER)
        assertNull(guard.state.held, "nothing here was destructive")
        assertEquals(
            listOf(Esc.ESCAPE + Esc.ENTER),
            io.text,
            "alt+enter reached the pane as an Escape and then an Enter",
        )
    }

    @Test
    fun aCommandTheGuardLetsThroughIsNotCutInTwoEither() {
        val pane = screen(80, "\$ ")
        val (io, sink, guard) = rig(pane)
        sink.raw("ls" + Esc.ENTER)
        assertNull(guard.state.held)
        assertEquals(listOf("ls" + Esc.ENTER), io.text)
    }

    @Test
    fun confirmingSendsExactlyWhatWasHeld() {
        val pane = screen(80, "\$ rm -rf build")
        val (io, sink, guard) = rig(pane)
        sink.raw("x" + Esc.ENTER)
        val held = assertNotNull(guard.state.held)
        assertEquals(listOf("x"), io.text)
        sink.confirmed(held.payload)
        assertEquals(listOf("x", Esc.ENTER), io.text)
    }
}
