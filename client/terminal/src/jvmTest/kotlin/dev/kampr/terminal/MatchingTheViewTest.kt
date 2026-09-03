package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.runDesktopComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.ui.LocalMosaicCell
import dev.kampr.shared.ui.LocalPaneChrome
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneChrome
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.ui.keyboardInset
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.MIN_PANE_COLS
import dev.kampr.shared.wire.MIN_PANE_ROWS
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.SizeMode
import dev.kampr.terminal.view.TerminalView
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

// A desk. Above `Breakpoint.Desktop`'s 900x600 dp on both axes, which is the line the default
// turns on (ADR 0013).
private val DESK = 1624.dp to 1000.dp

// Wide enough and tall enough to show a pane above the 80x24 floor, and still not a desk — the
// case that separates the two halves of the gate.
private val NEARLY = 899.dp to 900.dp

private val PHONE = 411.dp to 914.dp

private class MatchIo(private val stored: PanePrefs = PanePrefs()) : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }

    override fun prefs(paneId: String) = stored
}

private fun ClientMsg.sizing(): ManageOp.PaneSize? =
    ((this as? ClientMsg.Manage)?.request as? ManageOp.PaneSize)

private fun List<ClientMsg>.sizings() = mapNotNull { it.sizing() }

private fun grid(cols: Int): PaneState {
    val pane = PaneState(Phone.PANE, StyleTable())
    val line = "$ ls"
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = cols,
            rows = 40,
            rowsData = listOf(RowDiff(0, listOf(Run(0, line)))),
            cursor = Cursor(line.length, 0, true),
            links = emptyList(),
        ),
    )
    return pane
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.terminal(
    pane: PaneState,
    io: PaneIo,
    size: Pair<Dp, Dp>,
    session: PaneSession? = null,
    shown: () -> Boolean = { true },
) {
    setContent {
        CompositionLocalProvider(
            LocalTokens provides Phone.tokens(),
            LocalPaneIo provides io,
            LocalSafeArea provides Phone.BARS,
            LocalPaneChrome provides PaneChrome(Phone.HEADER),
        ) {
            Box(Modifier.size(size.first, size.second).keyboardInset()) {
                if (shown()) {
                    Box(Modifier.fillMaxSize()) {
                        TerminalView(pane, session ?: PaneSession(Phone.PANE), io)
                    }
                }
            }
        }
    }
    waitForIdle()
}

// Lets the clock run without waiting for anything, so a claim that would fire late has fired.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.quiet(millis: Long) {
    try {
        waitUntil(timeoutMillis = millis) { false }
    } catch (_: Throwable) {
        // The timeout is the point.
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.settled(io: MatchIo, mode: SizeMode): ManageOp.PaneSize? {
    // The claim is behind `MATCH_SETTLE_MS`, so a single `waitForIdle` proves nothing either way.
    try {
        waitUntil(timeoutMillis = 3_000) { io.sent.sizings().any { it.mode == mode } }
    } catch (_: Throwable) {
        return null
    }
    return io.sent.sizings().last { it.mode == mode }
}

// The one automatic claim in the product, and the four things that keep it inside rule 3:
// it is the terminal surface only, it is desk-sized only, it lets go when the view does, and an
// operator who said no is not asked again. See ADR 0013.
@OptIn(ExperimentalTestApi::class)
class MatchingTheViewTest {
    @Test
    fun aDeskSizedTerminalHoldsThePaneAtTheSizeItCanShow() = runComposeUiTest {
        val io = MatchIo()
        terminal(grid(cols = 40), io, DESK)
        val asked = assertNotNull(settled(io, SizeMode.Match), "a desk asked for nothing: ${io.sent}")
        assertTrue(
            (asked.cols ?: 0) >= MIN_PANE_COLS && (asked.rows ?: 0) >= MIN_PANE_ROWS,
            "a claim below the node's own floor would be refused every time: $asked",
        )
        assertEquals(Phone.PANE, asked.at)
    }

    // The half of the gate that is not the floor. This window would show a pane well above 80x24
    // and still is not a desk — a phone in landscape, a half-screen window, a mosaic cell.
    @Test
    fun aWindowThatIsNotDeskSizedHoldsNothingEvenThoughThePaneWouldFit() = runComposeUiTest {
        val io = MatchIo()
        terminal(grid(cols = 40), io, NEARLY)
        assertNull(settled(io, SizeMode.Match), "a window under desk size claimed a pane: ${io.sent}")
    }

    @Test
    fun aPhoneNeverHoldsAPaneWithoutBeingAsked() = runComposeUiTest {
        val io = MatchIo()
        terminal(grid(cols = 40), io, PHONE)
        assertNull(settled(io, SizeMode.Match), "a phone claimed a pane: ${io.sent}")
    }

    // Leaving the terminal for the conversation is this composable leaving the composition, which
    // is the same event as the pane closing and as the window going away. The node covers the case
    // where nothing leaves anything because the client stopped existing.
    @Test
    fun leavingTheTerminalLetsTheHoldGo() = runComposeUiTest {
        val io = MatchIo()
        var showing by mutableStateOf(true)
        terminal(grid(cols = 40), io, DESK) { showing }
        assertNotNull(settled(io, SizeMode.Match), "nothing was held, so nothing is being released")

        showing = false
        waitForIdle()
        assertNotNull(
            io.sent.sizings().lastOrNull { it.mode == SizeMode.Release },
            "the view closed still holding the pane: ${io.sent}",
        )
    }

    // The switch is stored per pane per device, and it wins over the size of the screen. An
    // operator who turned it off has turned it off.
    @Test
    fun aPaneTheOperatorTurnedMatchingOffForIsNotHeldOnADesk() = runComposeUiTest {
        val io = MatchIo(PanePrefs(mapOf("match" to "off")))
        terminal(grid(cols = 40), io, DESK)
        assertNull(settled(io, SizeMode.Match), "a pane switched off was claimed anyway: ${io.sent}")
    }

    // A mosaic cell on a wide desktop measures as a desk — two tiles on a 1920 px screen are
    // 960 px each — and a pane in a grid of thumbnails is not the thing being looked at. Nothing
    // that reaches the pane itself may fire from one.
    @Test
    fun aMosaicCellDoesNotHoldThePaneEvenWhenTheCellIsDeskSized() = runComposeUiTest {
        val io = MatchIo()
        setContent {
            CompositionLocalProvider(
                LocalTokens provides Phone.tokens(),
                LocalPaneIo provides io,
                LocalSafeArea provides Phone.BARS,
                LocalPaneChrome provides PaneChrome(Phone.HEADER),
                LocalMosaicCell provides true,
            ) {
                Box(Modifier.size(DESK.first, DESK.second).keyboardInset()) {
                    Box(Modifier.fillMaxSize()) {
                        TerminalView(grid(cols = 40), PaneSession(Phone.PANE), io)
                    }
                }
            }
        }
        waitForIdle()
        assertNull(settled(io, SizeMode.Match), "a tile in a grid claimed a pane: ${io.sent}")
    }

    // The switch travels with the pane, not with the screen — an operator who turned matching on
    // at their desk opens the same pane on a phone. The node's floor would refuse 52x30 every time,
    // and an op that is always refused is a toast the operator cannot act on.
    @Test
    fun aPaneSwitchedOnIsStillNotHeldFromAViewTooSmallToAsk() = runComposeUiTest {
        val io = MatchIo(PanePrefs(mapOf("match" to "on")))
        terminal(grid(cols = 40), io, PHONE)
        assertNull(
            settled(io, SizeMode.Match),
            "a view of ${PHONE.first} asked for a pane it could not be given: ${io.sent}",
        )
    }

    // **The proof that two viewers cannot take turns.** A claim is edge-triggered by this view —
    // it opened, it closed, the window changed shape — and by nothing the node says about the pane.
    // Granting one is what would otherwise start the loop: the pane arrives at its new width, that
    // is a change, the change is another claim, and two desks matching the same pane trade it back
    // and forth for ever. Counted rather than compared, because a second claim for the same size
    // is still a second `herdr terminal session control` child and still a second edge.
    @Test
    fun aPaneArrivingAtTheWidthItWasAskedForDoesNotStartAnotherAsk() = runComposeUiTest {
        val io = MatchIo()
        val pane = grid(cols = 40)
        terminal(pane, io, DESK)
        val first = assertNotNull(settled(io, SizeMode.Match), "nothing was asked for: ${io.sent}")

        pane.applyReset(
            ServerMsg.GridReset(
                pane = Phone.PANE,
                cols = first.cols!!,
                rows = first.rows!!,
                rowsData = listOf(RowDiff(0, listOf(Run(0, "$ ls")))),
                cursor = Cursor(4, 0, true),
                links = emptyList(),
            ),
        )
        waitForIdle()
        quiet(2_000)

        val asks = io.sent.sizings().filter { it.mode == SizeMode.Match }
        assertEquals(
            1,
            asks.size,
            "the pane moving asked again, which is the loop: $asks",
        )
        assertTrue(
            asks.all { it.cols == first.cols && it.rows == first.rows },
            "and it asked for something else: $asks",
        )
    }

    // **The two controls on the panel are one promise, so they are one number.** The chip measured
    // the window in cells of whatever size the operator was reading at while the switch measured it
    // at the base cell, and the pair therefore agreed only at 1x. On a grid wider than the window
    // the fit ladder is at the zoom that pane's width chose, so the chip was offering the pane
    // roughly the width it already had — and with the switch on, the standing hold undid it a
    // moment later.
    //
    // A real window rather than an oversized `Box`, because this is the one test here that presses
    // something: a control laid out past the edge of the test window cannot be clicked.
    @Test
    fun theChipAndTheSwitchAskForTheSameGridAtEveryZoom() {
        for (zoom in listOf("0.6", "1.0", "1.6")) {
            runDesktopComposeUiTest(DESK.first.value.toInt(), DESK.second.value.toInt()) {
                val io = MatchIo(PanePrefs(mapOf("zoom" to zoom)))
                val session = PaneSession(Phone.PANE)
                terminal(grid(cols = 300), io, DESK, session)
                val held = assertNotNull(settled(io, SizeMode.Match), "nothing was held at ${zoom}x")

                session.view.sheetOpen = true
                waitForIdle()
                onNodeWithContentDescription("Match this view ·", substring = true).performClick()
                waitForIdle()

                val once = assertNotNull(
                    io.sent.sizings().lastOrNull { it.mode == SizeMode.Once },
                    "the chip asked for nothing at ${zoom}x",
                )
                assertEquals(
                    held.cols to held.rows,
                    once.cols to once.rows,
                    "at ${zoom}x the chip and the switch named different grids",
                )
            }
        }
    }
}
