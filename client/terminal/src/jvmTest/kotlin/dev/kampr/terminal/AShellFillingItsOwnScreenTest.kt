package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.ScrollWheel
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.view.CARET_SETTLE_MS
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The operator's repro on the wasm desk, verbatim: *"open session, df -h until the screen is full,
// it jumps in various ways — sometimes it jumps above the empty new terminal, sometimes it jumps
// up the previous df -h command line, both times it stays there"*.
//
// **The jump is the size of whatever scrolled off, and it accumulates.** `TerminalView` derives
// `originY` — where row zero of the surface is painted — from `rows.total`, which is
// `historyRows + liveRows`; the renderer indexes rows against that origin inside a `drawBehind`
// that reads `pane.revision` and so redraws on every frame the ring grows. `historyRows` was a
// plain `var`, so the ring growing invalidated the *draw* and never the *composition*: the picture
// was painted a whole batch too low against an origin belonging to the shorter surface, and the
// next batch added to it. Nothing brought it back, which is the "it stays there".
//
// **A shell whose screen is full is the pane where nothing else recomposes it.** The body reads
// exactly one piece of snapshot state about the pane's shape — `pane.cursor` — and once the grid
// is full every `df -h` leaves the prompt on the same row at the same column, so the cursor is
// equal to the one before it and invalidates nothing. Below a full screen the caret walks down and
// hides the defect entirely, which is why it takes four commands to appear.
private val DESK = 1600.dp to 900.dp

private const val COLS = 94
private const val GRID_ROWS = 33

private const val PROMPT = "[20:36:31 dbrain@comingclean kampr]$ "

private val DF = listOf(
    "Filesystem      Size  Used Avail Use% Mounted on",
    "dev             7.8G     0  7.8G   0% /dev",
    "run             7.8G  1.9M  7.8G   1% /run",
    "/dev/nvme0n1p2  457G  310G  124G  72% /",
    "tmpfs           7.8G  221M  7.6G   3% /dev/shm",
    "/dev/nvme0n1p1  511M  288K  511M   1% /boot",
    "tmpfs           1.6G  132K  1.6G   1% /run/user/1000",
)

private const val PER_RUN = 8

// The shell the operator is typing into, as the node publishes it: the live grid is the last
// [GRID_ROWS] lines of the record and everything above them is the ring. A command that scrolls
// rewrites every row of the grid, and the rows that fell off it follow in a scrollback frame —
// which is the order the node sends the two in.
private class Shell {
    val record = mutableListOf(PROMPT)

    val history: Int get() = (record.size - GRID_ROWS).coerceAtLeast(0)
    val caretRow: Int get() = record.size - 1 - history

    fun df() {
        record[record.size - 1] = PROMPT + "df -h"
        record += DF
        record += PROMPT
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.publish(pane: PaneState, shell: Shell) {
    pane.applyPatch(
        ServerMsg.GridPatch(
            pane = Phone.PANE,
            rows = (0 until GRID_ROWS).map { row ->
                RowDiff(row, listOf(Run(0, shell.record.getOrElse(shell.history + row) { "" })))
            },
            cursor = Cursor(PROMPT.length, shell.caretRow, true),
            links = emptyList(),
        ),
    )
    if (shell.history > 0) {
        pane.applyScrollback(
            ServerMsg.Scrollback(
                pane = Phone.PANE,
                fromTop = 0,
                rows = (0 until shell.history).map { RowDiff(it, listOf(Run(0, shell.record[it]))) },
                totalRows = shell.history,
                complete = true,
                capped = false,
            ),
        )
    }
    mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
    waitForIdle()
}

// A pane the way a session opens on the desk: the desk's grid, one prompt on the first row, and no
// ring at all.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.aNewSession(): Triple<PaneState, PaneSession, Shell> {
    val pane = PaneState(Phone.PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = COLS,
            rows = GRID_ROWS,
            rowsData = listOf(RowDiff(0, listOf(Run(0, PROMPT)))),
            cursor = Cursor(PROMPT.length, 0, true),
            links = emptyList(),
        ),
    )
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width = DESK.first, height = DESK.second)
    mainClock.advanceTimeBy(CARET_SETTLE_MS * 2)
    waitForIdle()
    return Triple(pane, session, Shell())
}

// Where a row of the whole surface is painted, which is what the renderer does with the same two
// numbers — history and the live grid are one index space.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.surfaceRowTop(session: PaneSession, index: Int): Dp =
    with(density) { (session.grid.originY + index * session.grid.cellHeight).toDp() }

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.wheelUp(notches: Int) {
    onRoot().performMouseInput {
        moveTo(Offset(width / 2f, height / 2f))
        repeat(notches) { scroll(-1f, ScrollWheel.Vertical) }
    }
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
class AShellFillingItsOwnScreenTest {
    @Test
    fun a_shell_filling_its_own_screen_does_not_walk_the_viewport_into_its_history() =
        runComposeUiTest {
            val (pane, session, shell) = aNewSession()
            val view = session.view

            repeat(8) { turn ->
                shell.df()
                publish(pane, shell)
                val cell = session.grid.cellHeight
                assertTrue(
                    view.following,
                    "turn $turn: nothing but df -h has happened and the viewport stopped following",
                )
                // The prompt is the row being typed at. Off the bottom of the content rectangle is
                // where a surface drawn against too short an origin puts it, one batch further per
                // command, and the rows on screen are then the ones that scrolled off.
                assertTrue(
                    onScreen(pane, session, shell.caretRow),
                    "turn $turn: the prompt is off the screen — ${shell.history} rows of history, " +
                        "the surface drawn from ${session.grid.originY / cell} rows, the prompt at " +
                        "${rowTop(pane, session, shell.caretRow)} with the chrome at ${stripTop()}",
                )
            }
        }

    // The same blindness by its other face, and the reason it is not enough to fix the follower.
    // `carryHistory` is driven from the composition too: rows entering the ring extend the surface
    // *below* a reader parked above it, so they have to be moved down by exactly that much to stay
    // on the lines they are reading. A ring that grows without recomposing is a carry that never
    // happens, and the operator's own rows slide up the pane at the rate it produces output.
    @Test
    fun output_that_scrolls_without_moving_the_caret_still_carries_a_reader_in_the_history() =
        runComposeUiTest {
            val (pane, session, shell) = aNewSession()
            val view = session.view
            repeat(20) { shell.df() }
            publish(pane, shell)

            wheelUp(40)
            val parked = view.scrollY
            assertTrue(
                !view.following && parked > session.grid.cellHeight * GRID_ROWS,
                "the wheel has to leave the reader a screenful and more into the history, or the " +
                    "carry does not apply and nothing here is tested",
            )
            val reading = 10
            val was = surfaceRowTop(session, reading)

            repeat(3) {
                shell.df()
                publish(pane, shell)
            }

            // **Asked of the carry and not only of the pixel.** With the ring invisible to the
            // composition, `originY` is frozen along with everything else — so a row's position on
            // the screen sits still for exactly the reason the defect exists, and asserting on it
            // alone is a harness that was never the app (#191). What has to have happened is that
            // the surface grew and the reader was moved down it by the same amount.
            val carried = 3 * PER_RUN * session.grid.cellHeight
            assertEquals(
                parked + carried,
                view.scrollY,
                0.51f,
                "${3 * PER_RUN} rows entered the ring beneath a reader parked " +
                    "${parked / session.grid.cellHeight} rows up and the surface never carried them",
            )
            assertEquals(
                was.value,
                surfaceRowTop(session, reading).value,
                0.51f,
                "row $reading slid ${surfaceRowTop(session, reading) - was} up the screen while " +
                    "${3 * PER_RUN} rows entered the ring beneath it",
            )
        }
}
