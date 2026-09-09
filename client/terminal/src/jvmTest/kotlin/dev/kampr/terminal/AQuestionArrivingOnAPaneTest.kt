package dev.kampr.terminal

import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals

// **An agent asking a question is not a reason to reshape the operator's pane.**
//
// The terminal surface inset its scrollable content by 52 dp whenever `pane.pending` was set, for
// an answer strip that `PaneScreen` says in as many words it does not draw: *"No answer chips
// here, on purpose. The dialog is on the grid a few rows up, and the key row under it types into
// the pane."* `PendingStrip` is the conversation's, and always was — the inset dates to the first
// renderer commit and never had a bar under it.
//
// What it cost is two things at once. The band of nothing is visible — the operator, on the wasm
// desk: *"note the gap between the cols + the bottom bar"* — and, worse, the content rectangle is
// what `viewRows` is counted in, so a question appearing took three rows off the view and the
// standing hold claimed the pane at the smaller size. An agent's dialog opening and closing is
// then two reshapes of a pane nobody asked to resize, which is the one thing rule 3 forbids.
private val DESK = 1600.dp to 900.dp

private const val COLS = 94
private const val GRID_ROWS = 33

private class Claims : PaneIo {
    var cols = 0
    var rows = 0
    var claims = 0

    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
    override fun claimMatch(paneId: String, cols: Int, rows: Int): Boolean {
        this.cols = cols
        this.rows = rows
        claims++
        return true
    }
}

private fun question(pane: String) = ServerMsg.Pending(
    pane = pane,
    question = "Do you want to make this edit to PaneState.kt?",
    options = listOf(PendingOption("1", "Yes"), PendingOption("2", "No, tell Claude what to do")),
    source = "transcript",
)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.aFullPane(io: Claims): Triple<KamprStore, PaneState, PaneSession> {
    val store = KamprStore()
    val pane = store.pane(Phone.PANE)
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = COLS,
            rows = GRID_ROWS,
            rowsData = (0 until GRID_ROWS).map { RowDiff(it, listOf(Run(0, "output line $it"))) },
            cursor = Cursor(0, GRID_ROWS - 1, true),
            links = emptyList(),
        ),
    )
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width = DESK.first, height = DESK.second, io = io)
    mainClock.advanceTimeBy(2_000)
    waitForIdle()
    return Triple(store, pane, session)
}

@OptIn(ExperimentalTestApi::class)
class AQuestionArrivingOnAPaneTest {
    @Test
    fun a_question_arriving_does_not_take_rows_off_the_view_or_move_the_grid() = runComposeUiTest {
        val io = Claims()
        val (store, pane, session) = aFullPane(io)
        val heldAt = io.rows
        val lastRow = rowTop(pane, session, GRID_ROWS - 1)
        assertEquals(1, io.claims, "the pane has to have been claimed once, or nothing is tested")

        store.accept(question(Phone.PANE))
        mainClock.advanceTimeBy(2_000)
        waitForIdle()

        assertEquals(
            heldAt,
            io.rows,
            "an agent's question took ${heldAt - io.rows} rows off the view, and the standing " +
                "hold reshaped the operator's pane to match",
        )
        assertEquals(
            lastRow.value,
            rowTop(pane, session, GRID_ROWS - 1).value,
            0.51f,
            "the grid moved ${rowTop(pane, session, GRID_ROWS - 1) - lastRow} when the question " +
                "arrived — the surface is standing off a bar nothing draws",
        )
    }
}
