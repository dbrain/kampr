package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.runDesktopComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.ui.LocalConnectionStatus
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
import dev.kampr.terminal.view.ZoomButton
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
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

private class MatchIo(
    private val stored: PanePrefs = PanePrefs(),
    override val growsPanes: Boolean = false,
) : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }

    override fun prefs(paneId: String) = stored
}

// The same recorder, watching the two calls a session owns rather than the wire underneath them.
// A view that goes on sending the ops itself cannot be given a linger, and the pane switch that
// bounced (ADR 0013, `MatchHolds`) is exactly a view ending and another starting.
private class SessionIo(
    private val stored: PanePrefs = PanePrefs(),
    // Whether the node takes the pane. A claim is `pane.size` and it can be refused — herdr allows
    // one controller at a time and refuses the second outright (#21).
    private val takes: Boolean = true,
    override val growsPanes: Boolean = false,
) : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    val claims = mutableListOf<Triple<String, Int, Int>>()
    val grows = mutableListOf<Boolean>()
    val releases = mutableListOf<Pair<String, Boolean>>()

    override fun send(msg: ClientMsg) {
        sent += msg
    }

    override fun prefs(paneId: String) = stored

    override suspend fun claimMatch(paneId: String, cols: Int, rows: Int, grow: Boolean): Boolean {
        claims += Triple(paneId, cols, rows)
        grows += grow
        return takes
    }

    override fun releaseMatch(paneId: String, linger: Boolean) {
        releases += paneId to linger
    }
}

private val SAYS_HELD = SemanticsMatcher("says the pane is held") {
    it.config.getOrNull(SemanticsProperties.StateDescription)?.contains("holding this pane") == true
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.saysHeld(): Boolean =
    onAllNodes(SAYS_HELD).fetchSemanticsNodes().isNotEmpty()

private fun ClientMsg.sizing(): ManageOp.PaneSize? =
    ((this as? ClientMsg.Manage)?.request as? ManageOp.PaneSize)

private fun List<ClientMsg>.sizings() = mapNotNull { it.sizing() }

private fun grid(cols: Int, rows: Int = 40): PaneState {
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

// Claude Code's shape, measured against herdr 0.8.2 at 40 and at 60 rows: a logo on rows 2-4, a
// blank middle, and a composer anchored to the **last** row of the grid with the caret three rows
// above it. Nothing is blank tail — every row of a grid this program is given is one it has drawn
// on — which is what makes a grid too tall for its rectangle visible as rows off the top rather
// than as nothing at all.
private fun anchoredToTheLastRow(cols: Int, rows: Int) = ServerMsg.GridReset(
    pane = Phone.PANE,
    cols = cols,
    rows = rows,
    rowsData = listOf(
        RowDiff(1, listOf(Run(0, " ▐▛███▛█   Claude Code v2.1.260"))),
        RowDiff(rows - 5, listOf(Run(0, "─".repeat(cols - 1)))),
        RowDiff(rows - 4, listOf(Run(0, "❯ Try \"fix lint errors\""))),
        RowDiff(rows - 3, listOf(Run(0, "─".repeat(cols - 1)))),
        RowDiff(rows - 1, listOf(Run(0, "  ⏵⏵ auto mode on (shift+tab to cycle)"))),
    ),
    cursor = Cursor(2, rows - 4, true),
    links = emptyList(),
)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.terminal(
    pane: PaneState,
    io: PaneIo,
    size: Pair<Dp, Dp>,
    session: PaneSession? = null,
    status: () -> ConnectionStatus = { ConnectionStatus.Live("full") },
    header: Boolean = false,
    safe: () -> SafeArea = { Phone.BARS },
    shown: () -> Boolean = { true },
) {
    setContent {
        CompositionLocalProvider(
            LocalTokens provides Phone.tokens(),
            LocalPaneIo provides io,
            LocalSafeArea provides safe(),
            LocalPaneChrome provides PaneChrome(Phone.HEADER),
            LocalConnectionStatus provides status(),
        ) {
            Box(Modifier.size(size.first, size.second).keyboardInset()) {
                if (shown()) {
                    Box(Modifier.fillMaxSize()) {
                        TerminalView(pane, session ?: PaneSession(Phone.PANE), io)
                    }
                }
                if (header && session != null) ZoomButton(session, Modifier.align(Alignment.TopEnd))
            }
        }
    }
    waitForIdle()
}

// How many columns this machine's monospace font leaves on the desk at the base cell. Every zoom
// either test below reaches for is a ratio of it: the cell is whatever font the runner resolves,
// and a zoom named as a number is a different number of columns on every machine that runs it.
private const val COMFORTABLY_ABOVE_THE_FLOOR = 100f

@OptIn(ExperimentalTestApi::class)
private fun colsThisDeskShowsAtOne(): Int {
    var cols = 0
    runComposeUiTest {
        val io = MatchIo(PanePrefs(mapOf("zoom" to "1.0")))
        terminal(grid(cols = 94), io, DESK)
        cols = assertNotNull(settled(io, SizeMode.Match), "a desk held nothing at 1.0x").cols!!
    }
    assertTrue(cols > MIN_PANE_COLS, "a desk shows $cols columns at 1.0x, which is under the floor")
    return cols
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
    // **A socket dying is how a matched hold ends** — ADR 0013 point 1, and what
    // `a_matched_pane_is_put_back_when_the_socket_holding_it_stops_answering` proves from the
    // node's side: the lease goes with the socket and the pane is put back to the geometry it was
    // found at. So a reconnect arrives at a pane that is no longer held.
    //
    // Nothing asked for it again. The claim's keys are the pane id and the *view's* geometry, and
    // a reconnect moves neither; `claimed` is `remember(paneId)` and stays true, so the strip went
    // on saying the desk sees this pane at this shape for the whole outage and after it. Rule 3
    // wants the hold visible wherever it is held, and this made it visible where it was not.
    //
    // The mutation that must fail: drop the connection from the effect's keys, and the second
    // claim never goes.
    @Test
    fun aReconnectClaimsThePaneAgainRatherThanGoingOnBelievingItHoldsOne() = runComposeUiTest {
        val io = SessionIo()
        var status: ConnectionStatus by mutableStateOf(ConnectionStatus.Live("full"))
        terminal(grid(cols = 94), io, DESK, status = { status })
        quiet(1_000)
        assertEquals(1, io.claims.size, "a desk did not claim the pane at all: ${io.claims}")

        status = ConnectionStatus.Offline("the wifi went", 1_000)
        quiet(400)
        status = ConnectionStatus.Live("full")
        quiet(1_000)

        assertEquals(
            2,
            io.claims.size,
            "the pane was never claimed again after the socket came back, so a desk-sized window " +
                "shows a pane at its own geometry while the strip says it is held: ${io.claims}",
        )
    }

    // **A claim the node refused is asked again**, because the view that asked is still open and
    // still the size it was. Nothing else would ask: the claim's keys are the pane and the view's
    // own geometry, and a refusal moves neither — so the operator sat looking at a pane the desk's
    // size under a window that believed it had already been given its own, until they resized it
    // by hand (0.1.80). What must *not* do the asking is a loop against what the node reports
    // about the pane; ADR 0013 point 2 is why.
    //
    // The mutation that must fail: take `claimMatch`'s answer for granted, and the second ask
    // never goes.
    @Test
    fun aClaimTheNodeRefusedIsAskedAgainRatherThanTakenForAHold() = runComposeUiTest {
        val io = SessionIo(takes = false)
        terminal(grid(cols = 94), io, DESK)
        quiet(3_500)

        assertTrue(
            io.claims.size > 1,
            "a refused claim was never asked again, so the pane stays whatever the desk left it " +
                "at while the view says it is held: ${io.claims}",
        )
        assertEquals(io.claims.first(), io.claims.last(), "it asked for something else: ${io.claims}")
    }

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
    fun aWindowThatIsNotDeskSizedDoesNotHoldThePaneAtItsOwnSize() = runComposeUiTest {
        val io = MatchIo()
        terminal(grid(cols = 40), io, NEARLY)
        assertNull(settled(io, SizeMode.Match), "a window under desk size claimed a pane: ${io.sent}")
    }

    @Test
    fun aPhoneNeverHoldsAPaneAtItsOwnSize() = runComposeUiTest {
        val io = MatchIo(growsPanes = true)
        terminal(grid(cols = 40), io, PHONE)
        assertNull(settled(io, SizeMode.Match), "a phone claimed a pane at its own size: ${io.sent}")
    }

    // The operator's report, on a phone: a Claude pane *"counts the terminal as half the size of
    // the screen, so scrolling up scrolls just a portion of the screen"*. Their decision: *"only if
    // current size is smaller … don't shrink anything, only enlarge the dimension that is
    // smaller."* So the phone asks for its view as a `grow`, and the node — which is where the
    // pane's honest width is — enlarges only what the pane is short of: here the rows, never the
    // 300 columns down to a phone's.
    @Test
    fun aPhoneTallerThanThePaneAsksToGrowIt() = runComposeUiTest {
        val io = MatchIo(growsPanes = true)
        terminal(grid(cols = 300, rows = 10), io, PHONE)
        val asked = assertNotNull(settled(io, SizeMode.Grow), "a phone taller than the pane asked nothing: ${io.sent}")
        assertEquals(Phone.PANE, asked.at)
        assertTrue(asked.rows!! > 10, "it asked for no more rows than the pane has: $asked")
        assertTrue(io.sent.sizings().none { it.mode == SizeMode.Match }, "a phone asked for its exact size: ${io.sent}")
    }

    @Test
    fun aPaneThePhoneAlreadyFitsInIsNotAskedFor() = runComposeUiTest {
        val io = MatchIo(growsPanes = true)
        terminal(grid(cols = 300, rows = 200), io, PHONE)
        assertNull(settled(io, SizeMode.Grow), "a pane larger than the phone both ways was claimed: ${io.sent}")
    }

    // A node that predates `grow` refuses it as an unknown mode, and a refusal is a strip over the
    // pane on a phone whose operator asked for nothing.
    @Test
    fun aNodeThatCannotGrowAPaneIsNotAskedTo() = runComposeUiTest {
        val io = MatchIo()
        terminal(grid(cols = 300, rows = 10), io, PHONE)
        assertNull(settled(io, SizeMode.Grow), "a node with no grow was asked for one: ${io.sent}")
    }

    @Test
    fun aPaneTheOperatorTurnedMatchingOffForIsNotGrownOnAPhone() = runComposeUiTest {
        val io = MatchIo(PanePrefs(mapOf("match" to "off")), growsPanes = true)
        terminal(grid(cols = 300, rows = 10), io, PHONE)
        assertNull(settled(io, SizeMode.Grow), "a pane switched off was grown anyway: ${io.sent}")
    }

    // The price rule 3 charges a claim nobody pressed, on a phone as on a desk: the header says the
    // pane is held, and the control saying it leads to the switch that lets it go.
    @Test
    fun aGrownPaneSaysSoOnItsHeaderAndTheSwitchLetsItGo() {
        runDesktopComposeUiTest(PHONE.first.value.toInt(), PHONE.second.value.toInt()) {
            val io = SessionIo(growsPanes = true)
            val session = PaneSession(Phone.PANE)
            terminal(grid(cols = 300, rows = 10), io, PHONE, session, header = true)
            try {
                waitUntil(timeoutMillis = 3_000) { saysHeld() }
            } catch (_: Throwable) {
                // Asserted below.
            }
            assertEquals(listOf(true), io.grows, "a phone did not ask to grow the pane: ${io.claims}")
            assertTrue(saysHeld(), "the pane is grown and its header says nothing about it")

            onNodeWithContentDescription("Zoom, currently", substring = true).performClick()
            waitForIdle()
            onNodeWithContentDescription("Match this view while it's open ·", substring = true)
                .performClick()
            waitForIdle()

            assertTrue(io.releases.any { it == Phone.PANE to false }, "the switch let nothing go: ${io.releases}")
            assertFalse(saysHeld(), "the header still says held after the operator let go")
        }
    }

    // **A pane that arrives grown is what the hold asked for, not a reason to let it go.** The gate
    // that asks nothing of a pane the phone already fits is read when the view asks, never against
    // the pane afterwards: read against the pane, the grown pane would release its own hold, the
    // release would put the pane back, and the short pane would be asked for again.
    @Test
    fun aPaneArrivingGrownKeepsItsHoldAndIsNotAskedForAgain() = runComposeUiTest {
        val io = SessionIo(growsPanes = true)
        val session = PaneSession(Phone.PANE)
        val pane = grid(cols = 300, rows = 10)
        terminal(pane, io, PHONE, session, header = true)
        quiet(1_000)
        val (_, _, rows) = assertNotNull(io.claims.singleOrNull(), "nothing was grown: ${io.claims}")

        pane.applyReset(anchoredToTheLastRow(300, rows))
        waitForIdle()
        quiet(1_500)

        assertEquals(1, io.claims.size, "the pane arriving grown asked again: ${io.claims}")
        assertTrue(io.releases.isEmpty(), "the pane arriving grown let its own hold go: ${io.releases}")
        assertTrue(saysHeld(), "the header stopped saying the grown pane is held")
    }

    // The operator: "keyboard up to type, finish typing and close it, now claude sees the top of
    // the terminal as part way down the terminal view until it realises and resizes after some
    // amount of time". The keyboard took rows off the view, the phone asked to grow the pane to
    // the shorter view, the node found that no taller than the pane was found at and let the hold
    // go — putting the pane back — and the keyboard going down grew it again seconds later. The
    // keyboard is not the view getting smaller; nothing about the pane moves for it.
    @Test
    fun theKeyboardComingAndGoingNeverMovesAGrownPane() = runComposeUiTest {
        val io = SessionIo(growsPanes = true)
        val pane = grid(cols = 300, rows = 10)
        var bars by mutableStateOf(Phone.BARS)
        terminal(pane, io, PHONE, PaneSession(Phone.PANE), header = true, safe = { bars })
        quiet(1_000)
        val (_, _, rows) = assertNotNull(io.claims.singleOrNull(), "nothing was grown: ${io.claims}")
        pane.applyReset(anchoredToTheLastRow(300, rows))
        waitForIdle()

        bars = Phone.KEYBOARD
        quiet(1_500)
        bars = Phone.BARS
        quiet(1_500)

        assertEquals(1, io.claims.size, "the keyboard asked for the pane again: ${io.claims}")
        assertTrue(io.releases.isEmpty(), "the keyboard let the hold go: ${io.releases}")
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

    // **The wiring, and it is the whole of the pane-switch fix.** The claim and the release go to
    // the session rather than onto the wire, because only the session can tell a view ending from
    // the operator answering — and a pane switch is a view ending. ADR 0013's release restores the
    // pane, so a view that released on its own way out resized the pane being left and the pane
    // being opened on every switch, and both again coming back.
    @Test
    fun the_hold_is_asked_for_and_let_go_of_through_the_session() = runComposeUiTest {
        val io = SessionIo()
        var showing by mutableStateOf(true)
        terminal(grid(cols = 40), io, DESK) { showing }
        try {
            waitUntil(timeoutMillis = 3_000) { io.claims.isNotEmpty() }
        } catch (_: Throwable) {
            // Asserted below with the claim list in the message.
        }
        assertTrue(io.claims.isNotEmpty(), "a desk claimed nothing through the session: ${io.sent}")
        assertEquals(Phone.PANE, io.claims.last().first)
        assertTrue(
            io.sent.sizings().none { it.mode == SizeMode.Match },
            "the view put a claim on the wire behind the session's back: ${io.sent}",
        )

        showing = false
        waitForIdle()
        assertEquals(
            listOf(Phone.PANE to true),
            io.releases,
            "a view ending has to linger — it is a pane switch as often as it is a pane left",
        )
        assertTrue(
            io.sent.sizings().none { it.mode == SizeMode.Release },
            "the view put a release on the wire behind the session's back: ${io.sent}",
        )
    }

    // And the one release that must not linger: the operator ticking the switch off is an answer
    // about this pane, so the pane goes back now.
    @Test
    fun turning_the_switch_off_gives_the_pane_back_without_the_linger() {
        runDesktopComposeUiTest(DESK.first.value.toInt(), DESK.second.value.toInt()) {
            val io = SessionIo()
            val session = PaneSession(Phone.PANE)
            terminal(grid(cols = 40), io, DESK, session)
            try {
                waitUntil(timeoutMillis = 3_000) { io.claims.isNotEmpty() }
            } catch (_: Throwable) {
                // Asserted below.
            }
            assertTrue(io.claims.isNotEmpty(), "nothing was held, so nothing is being let go of")

            session.view.sheetOpen = true
            waitForIdle()
            onNodeWithContentDescription("Match this view while it's open ·", substring = true)
                .performClick()
            waitForIdle()

            assertTrue(
                io.releases.any { it == Phone.PANE to false },
                "an untick was given a view switch's grace window: ${io.releases}",
            )
        }
    }

    // **A hold is said where it is held, and the thing saying it leads to the switch** — rule 3's
    // price for a claim nobody pressed. The pane's own zoom control carries it, because that is
    // the control that opens the panel the off switch lives on, and it is in every pane header.
    //
    // The mutation that must fail: stop setting the view's matched flag from the claim, and the
    // header never says a word while the desk is being overridden.
    @Test
    fun aHeldPaneSaysSoOnItsOwnHeaderAndThatLeadsToTheOffSwitch() {
        runDesktopComposeUiTest(DESK.first.value.toInt(), DESK.second.value.toInt()) {
            val io = SessionIo()
            val session = PaneSession(Phone.PANE)
            terminal(grid(cols = 40), io, DESK, session, header = true)
            try {
                waitUntil(timeoutMillis = 3_000) { saysHeld() }
            } catch (_: Throwable) {
                // Asserted below.
            }
            assertTrue(io.claims.isNotEmpty(), "a desk claimed nothing")
            assertTrue(saysHeld(), "the pane is held and its header says nothing about it")

            onNodeWithContentDescription("Zoom, currently", substring = true).performClick()
            waitForIdle()
            onNodeWithContentDescription("Match this view while it's open ·", substring = true)
                .performClick()
            waitForIdle()

            assertTrue(io.releases.any { it == Phone.PANE to false }, "the switch let nothing go: ${io.releases}")
            assertFalse(saysHeld(), "the header still says held after the operator let go")
        }
    }

    // And it never says so of a pane it is not holding: a claim the node refused, and a claim a
    // dropped socket has already given back (ADR 0013 point 1).
    @Test
    fun aPaneThatIsNotHeldIsNotSaidToBe() = runComposeUiTest {
        val refused = SessionIo(takes = false)
        terminal(grid(cols = 40), refused, DESK, PaneSession(Phone.PANE), header = true)
        quiet(1_000)
        assertTrue(refused.claims.isNotEmpty(), "nothing was asked for, so nothing was refused")
        assertFalse(saysHeld(), "a refused claim was said to be a hold")
    }

    @Test
    fun aHoldASocketGaveBackIsNotSaidToBeHeld() = runComposeUiTest {
        val io = SessionIo()
        var status: ConnectionStatus by mutableStateOf(ConnectionStatus.Live("full"))
        terminal(grid(cols = 40), io, DESK, PaneSession(Phone.PANE), status = { status }, header = true)
        quiet(1_000)
        assertTrue(saysHeld(), "nothing was held to begin with")
        status = ConnectionStatus.Offline("the wifi went", 60_000)
        waitForIdle()
        assertFalse(saysHeld(), "the header says held over a pane the node has already put back")
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

    // **What the hold promises is a pane the view can show, and the operator reads it as one.**
    // The report: "i open claude and it fits the pane but it leaves a few blank lines at the bottom
    // and i need to scroll up to see the claude logo top — once scrolled up it doesn't let me
    // scroll down (implying it knows the size)".
    //
    // A grid measured at the base cell and drawn at the operator's own is exactly that much too
    // tall for the rectangle drawing it: on this machine's font the desk was held at 131x32 and
    // could show 26 of those rows at 1.2x, so six sat above the header with the surface carrying
    // travel it should never have had. The pane the arithmetic is checked against is Claude Code's
    // own shape, measured on herdr 0.8.2: it anchors its composer to the last row of whatever grid
    // it is given, so there is no blank tail to lose the overflow in and every row of it is a row
    // of the record.
    //
    // **The magnified rung is derived rather than named**, because the cell is whatever font the
    // machine resolves for monospace and a zoom that is comfortably above the floor here is past it
    // on a runner. That is what `aViewZoomedPastTheFloor` failed on in CI, and the same hazard
    // `FileHarness` names about where a tapped cell lands.
    @Test
    fun aPaneHeldAtTheViewsSizeIsAPaneTheViewCanShow() {
        val atOne = colsThisDeskShowsAtOne()
        for (zoom in listOf(0.6f, 1f, atOne / COMFORTABLY_ABOVE_THE_FLOOR)) {
            runComposeUiTest {
                val io = MatchIo(PanePrefs(mapOf("zoom" to zoom.toString())))
                val session = PaneSession(Phone.PANE)
                val pane = grid(cols = 94)
                terminal(pane, io, DESK, session)
                val held = assertNotNull(settled(io, SizeMode.Match), "nothing was held at ${zoom}x")
                pane.applyReset(anchoredToTheLastRow(held.cols!!, held.rows!!))
                waitForIdle()
                quiet(1_000)

                assertEquals(
                    0f,
                    session.view.maxScroll,
                    session.grid.cellHeight,
                    "at ${zoom}x the pane was held at ${held.cols}x${held.rows} and overflows the " +
                        "view it was matched to by ${session.view.maxScroll}px — " +
                        "${session.view.maxScroll / session.grid.cellHeight} rows of it are off the screen",
                )
            }
        }
    }

    // The other end of the same number, and the rule it has to keep: a view that cannot show a pane
    // at ADR 0012's 80x24 floor does not ask for one. The honest answer there is to hold nothing and
    // let the surface scroll — never to hold the pane at a grid measured in cells nobody is reading
    // in. The zoom is the one that leaves this machine one column short of the floor, whatever its
    // font makes a cell.
    @Test
    fun aViewZoomedPastTheFloorHoldsNothingRatherThanHoldingWhatItCannotShow() {
        val past = colsThisDeskShowsAtOne() / (MIN_PANE_COLS - 1f)
        runComposeUiTest {
            val io = MatchIo(PanePrefs(mapOf("zoom" to past.toString())))
            val session = PaneSession(Phone.PANE)
            terminal(grid(cols = 94), io, DESK, session)
            // The presets clamp a stored zoom, and a clamp would put the view back above the floor
            // and pass this test for the wrong reason.
            assertEquals(
                past,
                session.view.zoom,
                0.01f,
                "the zoom this test needs was clamped to ${session.view.zoom}x",
            )
            assertNull(
                settled(io, SizeMode.Match),
                "a view showing ${MIN_PANE_COLS - 1} columns claimed a pane anyway: ${io.sent.sizings()}",
            )
        }
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
