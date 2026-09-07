package dev.kampr.terminal

import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertTrue

// The operator, on 0.1.60: *"i did a paru -Syu, it initially drew fine then started walking back up
// when CMD was done, two jumps up"*, and the same on Android after quitting `top`.
//
// **Two jumps is a signature.** Exactly two readings feed the floors the viewport is placed
// against — `settledBelow`, the rows under the caret, and `settledContent`, the rows under the end
// of the record — and they settle on independent `CARET_SETTLE_MS` timers. The cells are not
// snapshot state, so the canvas draws the new output at once ("initially drew fine") and the
// composition only re-runs when one of those two lands. Two landings, two band changes, two
// placements of a following surface.
//
// The shape here is the operator's, and it is the shape the earlier attempt at this missed: a
// **long ring** under a **phone-sized viewport**, so the surface is enormously taller than the
// window and neither floor is clamped by `maxScroll`.
//
// **And the prompt is where a command that scrolls leaves it**: near the bottom of the grid, with
// the record running down to it. The record's floor stops at the top of the pane's own grid (#502),
// so a record that fits inside the viewport has no floor left to move — which is the right answer
// for that pane and no test of a floor that is one reading stale. `paru -Syu` fills a screen long
// before it finishes, and the two settles this is about are the ones at the end of it.
//
// **The grid is deep for the same reason, and it is a fact about the runner rather than about the
// pane.** With a 69-row grid the cap is `69 - <rows the viewport holds>`, and how many rows a
// viewport holds is a function of the font the host resolved — which is not the same font on CI as
// on the machine this was written on. There the cap came out under the four rows this asserts and
// the floor stopped being the record's end at all, so it passed here and failed there for six runs.
// Two hundred rows puts the cap a hundred and fifty rows clear of the number under test on any
// geometry, which is what makes the assertion about the settle rather than about the runner.
private val PHONE = 411.dp to 914.dp
private val DESK = 1600.dp to 900.dp

private const val GRID_ROWS = 200
private const val PROMPT_AT = 192
private const val RING = 2000

private fun history(count: Int) = ServerMsg.Scrollback(
    pane = Phone.PANE,
    fromTop = 0,
    rows = (0 until count).map { RowDiff(it, listOf(Run(0, "scrollback row $it"))) },
    totalRows = count,
    complete = true,
    capped = false,
)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.aShellWithHistory(size: Pair<Dp, Dp>): Pair<PaneState, PaneSession> {
    val pane = Phone.shell(rows = GRID_ROWS, caretRow = PROMPT_AT)
    val session = PaneSession(Phone.PANE)
    phoneTerminal(pane, session, width = size.first, height = size.second)
    pane.applyScrollback(history(RING))
    // The ring is not snapshot state; the grid frame that carried it is what the surface sees.
    wrote(pane, PROMPT_AT, "[dbrain@giftofthemagi2 kobbler]$ paru -Syu")
    assertTrue(session.view.following, "a pane nobody has touched follows its own output")
    assertTrue(session.view.maxScroll > 0f, "the surface has to overflow, or nothing is tested")
    // The record's end has to be what the floor is made of. If the viewport of whatever host is
    // running this holds nearly the whole grid, the cap at the top of the pane (#502) is the lower
    // of the two and the floor stops tracking the record at all — which is correct behaviour and
    // no test of a settle. Named here so that failure says so, rather than looking like the
    // regression the assertion below is about.
    val uncapped = (GRID_ROWS - PROMPT_AT - 1) * session.grid.cellHeight
    assertTrue(
        kotlin.math.abs(session.view.contentFloor - uncapped) < 0.51f,
        "$size: this viewport holds so much of a $GRID_ROWS-row grid that the floor is the top of " +
            "the pane (${session.view.contentFloor}px) rather than the end of the record " +
            "(${uncapped}px) — the pane under test needs to be deeper than the window",
    )
    return pane to session
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.wrote(pane: PaneState, row: Int, text: String) {
    pane.applyPatch(
        ServerMsg.GridPatch(
            pane = Phone.PANE,
            rows = listOf(RowDiff(row, listOf(Run(0, text)))),
            cursor = Cursor(text.length, row, true),
            links = emptyList(),
        ),
    )
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
class WhenACommandIsDoneTest {
    @Test
    fun the_two_settles_after_a_command_finishes_do_not_walk_the_surface_up() {
        for (size in listOf(PHONE, DESK)) {
            runComposeUiTest {
                val (pane, session) = aShellWithHistory(size)
                val view = session.view

                wrote(pane, PROMPT_AT + 1, "sudo: The \"no new privileges\" flag is set")
                wrote(pane, PROMPT_AT + 2, "sudo: If sudo is running in a container, you may need")
                wrote(pane, PROMPT_AT + 3, "[dbrain@giftofthemagi2 kobbler]$ ")
                // **The floor is the half this pins.** It is the position at which the end of
                // the record sits on the bottom of the viewport, and until this fix it waited out
                // `CARET_SETTLE_MS` before it knew the record had grown — so for a fifth of a
                // second after every command that printed anything, the surface was held at a
                // floor belonging to the output before it and the newest lines were below the
                // fold. Three lines written is three rows of floor, at once.
                val floorNow = view.band.floor / session.grid.cellHeight
                val recordEnd = (GRID_ROWS - (PROMPT_AT + 3) - 1).toFloat()
                assertTrue(
                    kotlin.math.abs(floorNow - recordEnd) < 0.51f,
                    "$size: the floor still describes the output before this command — it is at " +
                        "$floorNow rows and the record now ends $recordEnd rows off the bottom",
                )
            }
        }
    }
}
