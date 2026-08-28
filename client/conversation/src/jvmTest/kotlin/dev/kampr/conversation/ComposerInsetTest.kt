package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.ui.named
import androidx.compose.foundation.layout.height
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.BottomEdgeHeldBelow
import dev.kampr.shared.ui.BottomNav
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.REPLY_ROOM
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.ui.Tab
import dev.kampr.shared.ui.keyboardInset
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertTrue

private val BARS = SafeArea(top = 32.dp, bottom = 24.dp)
private val KEYBOARD = BARS.copy(ime = 300.dp)
private const val REPLY = "Reply to claude"
private const val SEND = "Send this reply"

// Rotated: the gesture handle thins and a three-button bar leaves the bottom for one side.
// Measured on an emulator at 560 dpi, rotated, with the stock keyboard up: 411 dp of window, of
// which the keys take 212 dp and the pane's own header — which floats over the conversation and
// is paid for as a top padding — takes 83 dp more.
private val ROTATED_WINDOW = DpSize(694.dp, 411.dp)
private val ROTATED_KEYS = SafeArea(top = 24.dp, bottom = 0.dp, ime = 212.dp)
private val PANE_CHROME = 83.dp

private fun rotatedPane(pending: Boolean): PaneState {
    val store = KamprStore()
    store.accept(requireNotNull(Wire.decode(RICH_CONVO)) { "undecodable frame" })
    // A real question, not a three-word one: harnesses ask about a named file and offer to stop
    // asking, and those labels wrap the strip onto several rows.
    if (pending) {
        store.accept(
            ServerMsg.Pending(
                pane = PANE_ID,
                question = "Edit crates/kampr-core/src/width.rs?",
                options = listOf(
                    PendingOption("1", "Yes, make this edit"),
                    PendingOption("2", "Yes, and do not ask again for this file"),
                    PendingOption("3", "No, and tell Claude what to do differently"),
                ),
                source = "screen",
            )
        )
    }
    return store.pane(PANE_ID)
}

// Rotated: the gesture handle thins and a three-button bar leaves the bottom for one side.
private val ROTATED = listOf(
    SafeArea(top = 24.dp, bottom = 24.dp),
    SafeArea(top = 24.dp, bottom = 0.dp, left = 48.dp),
    SafeArea(top = 24.dp, bottom = 0.dp, right = 48.dp),
)

// What Android hangs off a selection: a 25 dp box below the line it grips, and another the same
// distance to the left of the character the selection starts at. Both are windows of their own,
// which is why no layout in this file can clip them and why room for them has to be left.
private val SELECTION_HANDLE = 25.dp

// The three shapes the reply box ends up in, and the difference between them is what is under it:
// the tab bar, the gesture handle, or the keyboard itself.
private data class Posture(
    val name: String,
    val safe: SafeArea,
    val nav: Boolean,
    val window: DpSize? = null,
    val chrome: Dp = 0.dp,
)

private val POSTURES = listOf(
    Posture("portrait, tab bar", BARS, nav = true),
    Posture("portrait, no tab bar", BARS, nav = false),
    Posture("rotated pane", ROTATED_KEYS, nav = false, window = ROTATED_WINDOW, chrome = PANE_CHROME),
)

// What the phone actually stacks: the app's root, the pane inside it, the bottom navigation under
// that — which is what holds the gesture handle off, and says so. The reply box is the last thing
// in the conversation and the first thing a keyboard covers.
//
// `nav = false` is the rotated pane, which wears no tab bar at all: there the reply box is what
// ends at the window and owes the handle itself.
@OptIn(ExperimentalTestApi::class)
@Composable
private fun Phone(safe: SafeArea, nav: Boolean = true, window: DpSize? = null, content: @Composable () -> Unit) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
        LocalSafeArea provides safe,
    ) {
        Box((if (window == null) Modifier.fillMaxSize() else Modifier.size(window)).keyboardInset()) {
            Column(Modifier.fillMaxSize()) {
                Box(Modifier.weight(1f)) { BottomEdgeHeldBelow(nav, content) }
                if (nav) BottomNav(Tab.Herd, {})
            }
        }
    }
}

@OptIn(ExperimentalTestApi::class)
class ComposerInsetTest {
    // The report, verbatim: "typing in conversation window doesn't show you the text entry box
    // (its under keyboard)". The window is not resized when the keyboard opens, so the reply box
    // stayed exactly where it was — 300 dp below the top of the keys. Both halves matter: a fix
    // that pushed it off the top instead would satisfy the first assertion and nothing else.
    @Test
    fun theReplyBoxStaysAboveTheKeyboardAndOnScreen() = runComposeUiTest {
        val (_, pane) = demoPane(RICH_CONVO)
        setContent { Phone(KEYBOARD) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) } }
        val screen = onRoot().getUnclippedBoundsInRoot()
        val reply = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
        assertTrue(
            reply.bottom <= screen.bottom - KEYBOARD.ime,
            "the reply box reaches ${reply.bottom}, and the keys start at ${screen.bottom - KEYBOARD.ime}",
        )
        assertTrue(reply.top >= screen.top, "the reply box starts at ${reply.top}, above the window")
        assertTrue(reply.bottom > reply.top, "the reply box measured ${reply.top}..${reply.bottom}")
    }

    // With the keyboard closed nothing moves, or every conversation loses a strip to a keyboard
    // that is not there.
    @Test
    fun aClosedKeyboardLeavesTheConversationWhereItWas() = runComposeUiTest {
        val (_, pane) = demoPane(RICH_CONVO)
        setContent { Phone(BARS) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) } }
        val screen = onRoot().getUnclippedBoundsInRoot()
        val nav = onNodeWithContentDescription("Herd tab").getUnclippedBoundsInRoot()
        assertTrue(
            nav.bottom <= screen.bottom && nav.bottom > screen.bottom - 100.dp,
            "the bottom navigation is at ${nav.bottom} of ${screen.bottom}",
        )
    }

    // The last turn is what you are replying to, and a lazy list anchors on its first visible item
    // — so when the keyboard halves the viewport the turn slides out from under you. The reply box
    // has to ride up with the keys, and the end of the transcript has to still be under it.
    @Test
    fun theTranscriptFollowsTheKeyboardUp() = runComposeUiTest {
        val (_, pane) = demoPane(RICH_CONVO)
        var safe by mutableStateOf(BARS)
        setContent { Phone(safe) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) } }
        waitForIdle()
        val closed = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
        assertTrue(lastTurnIsComposed(), "the transcript did not start at its end")
        safe = KEYBOARD
        waitForIdle()
        val open = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
        assertTrue(
            open.bottom <= closed.bottom - KEYBOARD.ime + BARS.ime,
            "the reply box was at ${closed.bottom} and the keyboard moved it only to ${open.bottom}",
        )
        assertTrue(lastTurnIsComposed(), "the last turn scrolled out when the keyboard opened")
    }

    // The third container, and the one nothing was holding: rotate the phone onto a pane and the
    // tab bar goes, so the reply box is the last thing in the window and the gesture handle lands
    // on it. Rotated with three-button navigation the bar takes a side instead, which is where the
    // send button is.
    @Test
    fun aRotatedPaneHasNothingUnderTheReplyBoxButTheReplyBox() {
        for (bars in ROTATED) {
            runComposeUiTest {
                val (_, pane) = demoPane(RICH_CONVO)
                setContent { Phone(bars, nav = false) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) } }
                val screen = onRoot().getUnclippedBoundsInRoot()
                val reply = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
                val send = onNodeWithContentDescription(SEND, substring = true).getUnclippedBoundsInRoot()
                assertTrue(
                    reply.bottom <= screen.bottom - bars.bottom,
                    "$bars: the reply box reaches ${reply.bottom} of ${screen.bottom}, inside the " +
                        "${bars.bottom} the system draws its gesture handle in",
                )
                assertTrue(
                    reply.left >= bars.left && send.right <= screen.right - bars.right,
                    "$bars: the composer spans ${reply.left}..${send.right} of ${screen.right}, " +
                        "inside the bar that the rotation moved to the side",
                )
            }
        }
    }

    // Reported from a phone: "I tap a word and the native android selection anchors show but they
    // are drawn off screen, as is the selection highlight." They were not translated. Rotated with
    // the keys up there are 411 dp of window, the keyboard takes 212 and the pane's own floating
    // header 83, and what is left had to hold a transcript bar, a question strip and a reply box
    // and could not. A column measures its unweighted children in order against what the ones
    // before them left, so the reply box — the last of them — was measured against nothing, came
    // back 0 dp tall, and was placed 22 dp past the bottom of the window. Android's handles hang
    // below the baseline of the text in it, so they were drawn under the keyboard.
    //
    // Both fixtures, because they are two different overflows: the plain pane is the one the
    // report came from, and a pane with a question open has a whole answer strip in the way too.
    @Test
    fun aRotatedPaneWithTheKeysUpShowsTheWholeReplyBox() {
        val settled = settledReplyHeight()
        for (pending in listOf(false, true)) {
            runComposeUiTest {
                val pane = rotatedPane(pending)
                setContent {
                    Phone(ROTATED_KEYS, nav = false, window = ROTATED_WINDOW) {
                        ConversationView(pane, demoInfo(), Modifier.fillMaxSize().padding(top = PANE_CHROME))
                    }
                }
                waitForIdle()
                val floor = ROTATED_WINDOW.height - ROTATED_KEYS.ime
                val reply = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
                val send = onNodeWithContentDescription(SEND, substring = true).getUnclippedBoundsInRoot()
                assertTrue(
                    reply.bottom - reply.top >= settled - 1.dp,
                    "pending=$pending: the reply box was squeezed to ${reply.bottom - reply.top}, " +
                        "against the $settled one line of it needs",
                )
                assertTrue(
                    reply.bottom <= floor && send.bottom <= floor,
                    "pending=$pending: the reply box ends at ${reply.bottom} and the send button at " +
                        "${send.bottom}, below the $floor the keys leave",
                )
            }
        }
    }

    // **The previous fix held for the one keyboard it was measured against and no taller one.**
    // `aRotatedPaneWithTheKeysUpShowsTheWholeReplyBox` only ever asks about the 212 dp keyboard
    // that produced the report; at 280 dp — an ordinary height for a keyboard carrying a
    // suggestion strip, or a larger font scale — the field measured 0 dp again (#319).
    //
    // Two things were in the way and only both together fix it, which is why this drives the real
    // `PaneScreenMobile` rather than standing in for it with a fixed top padding. The pane header
    // floats over the conversation and is paid for as that padding, so on a short window it took
    // room the reply box then could not have; and a column measures its unweighted children in
    // index order, so the reply box was last in the queue for what was left. A harness that pads
    // by a constant exercises neither and would go on passing with the header's half reverted —
    // which is #191, a harness that was never the app.
    //
    // Swept rather than pinned, because the defect is not at one height: it is at every height
    // past whichever one somebody happened to measure.
    @Test
    fun aRotatedPaneKeepsItsReplyBoxAtEveryKeyboardHeightAndNotJustTheMeasuredOne() {
        val settled = settledReplyHeight()
        for (ime in listOf(212.dp, 240.dp, 280.dp, 320.dp)) {
            runComposeUiTest {
                val keys = ROTATED_KEYS.copy(ime = ime)
                setContent {
                    Phone(keys, nav = false, window = ROTATED_WINDOW) {
                        PaneScreenMobile(
                            pane = rotatedPane(false),
                            info = demoInfo(),
                            view = PaneView.Conversation,
                            surfaces = ConversationSurfaces(),
                            landscape = true,
                            readOnly = false,
                            onBack = {},
                            onView = {},
                            onAnswer = {},
                            modifier = Modifier.fillMaxSize(),
                        )
                    }
                }
                waitForIdle()
                val floor = ROTATED_WINDOW.height - ime
                val reply = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
                assertTrue(
                    reply.bottom - reply.top >= settled - 1.dp,
                    "ime=$ime: the reply box measured ${reply.bottom - reply.top}, against the " +
                        "$settled one line of it needs — there is nothing to type into",
                )
                assertTrue(
                    reply.bottom <= floor,
                    "ime=$ime: the reply box ends at ${reply.bottom}, below the $floor the keys leave",
                )
            }
        }
    }

    // **Searching with the keys up is the one posture where the bar and the reply box compete**,
    // and it is why the conversation measures its children in priority order rather than in reading
    // order. Everywhere else the bar stands down on its own — it is composed only while the
    // keyboard is closed — so a plain column is indistinguishable from a correct one and the
    // header's half of the fix appears to be the whole of it. Open search, and the bar is there
    // with a keyboard under it.
    // **The one posture where the bar and the reply box compete for the same pixels.** Everywhere
    // else the bar stands down on its own — it is composed only while the keyboard is closed — so
    // a column that measures in reading order is indistinguishable from one that measures in
    // priority order, and the header's half of the fix looks like the whole of it. Searching keeps
    // the bar on screen with the keys up, and then the order decides who gets nothing.
    //
    // Driven at the layout rather than through the search field: opening search puts focus in a
    // text field, and flipping the keyboard under a focused field leaves `waitForIdle` spinning
    // for ever. The three children are given the heights the real ones measure, which is what the
    // rule is about — who is asked first, not what they contain.
    @Test
    fun theReplyBoxIsMeasuredBeforeTheBarAboveItAndNotAfter() = runComposeUiTest {
        val room = 96.dp
        setContent {
            CompositionLocalProvider(LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone, Ground.Dark)) {
                ConversationColumn(
                    Modifier.size(400.dp, room),
                    bar = { Box(Modifier.fillMaxWidth().height(40.dp).named("bar")) },
                    transcript = { Box(Modifier.fillMaxWidth().named("transcript")) },
                    composer = { Box(Modifier.fillMaxWidth().height(REPLY_ROOM).named("composer")) },
                )
            }
        }
        waitForIdle()
        val composer = onNodeWithContentDescription("composer").getUnclippedBoundsInRoot()
        assertTrue(
            composer.bottom - composer.top >= REPLY_ROOM - 1.dp,
            "in ${room} of column the reply box got ${composer.bottom - composer.top}, against the " +
                "$REPLY_ROOM it asked for — the bar above it was served first",
        )
    }

    // The room the pane header gives up is the room the reply box needs, so the two have to agree
    // on the number. This is the composer's side of it: if it grows past what `REPLY_ROOM` reserves,
    // the header yields too little and the sweep above starts failing at a height nobody changed.
    @Test
    fun aReplyBoxIsNoTallerThanTheRoomReservedForIt() = runComposeUiTest {
        setContent {
            Phone(BARS.copy(ime = 0.dp), nav = false) {
                Composer(agent = "claude", enabled = true, onSend = {}, modifier = Modifier)
            }
        }
        waitForIdle()
        val reply = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
        val send = onNodeWithContentDescription(SEND, substring = true).getUnclippedBoundsInRoot()
        val tallest = maxOf(reply.bottom, send.bottom) - minOf(reply.top, send.top)
        assertTrue(
            tallest <= REPLY_ROOM,
            "the reply box wants $tallest and the pane header only stands down by $REPLY_ROOM",
        )
    }

    // How tall one line of reply box is when nothing is squeezing it. Measured rather than named,
    // because what makes the report visible is the *difference*: a field shorter than its own line
    // of text clips that text through the middle, and Android draws the selection handles below
    // the baseline of a line that is no longer where it looks.
    private fun settledReplyHeight(): Dp {
        var height = 0.dp
        runComposeUiTest {
            setContent {
                Phone(BARS.copy(ime = 0.dp), nav = false) {
                    ConversationView(rotatedPane(false), demoInfo(), Modifier.fillMaxSize())
                }
            }
            waitForIdle()
            val reply = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
            height = reply.bottom - reply.top
        }
        return height
    }

    // The handles are drawn where the reply box is, in their own windows, and the rotated pane is
    // the posture where what is directly under the reply box is the keyboard. The room under the
    // field measured 26 dp against a 25 dp handle and the room beside it 28 — both of them the sum
    // of two paddings chosen for how they look, so the clearance was a coincidence that any retune
    // of the field's shape would have spent. All three postures, because only one of them is tight
    // and a rule written for that one has to hold for the other two.
    @Test
    fun theReplyBoxKeepsAHandlesWorthOfRoomUnderAndBesideIt() {
        for (posture in POSTURES) {
            runComposeUiTest {
                val (_, pane) = demoPane(RICH_CONVO)
                setContent {
                    Phone(posture.safe, nav = posture.nav, window = posture.window) {
                        ConversationView(pane, demoInfo(), Modifier.fillMaxSize().padding(top = posture.chrome))
                    }
                }
                waitForIdle()
                val screen = onRoot().getUnclippedBoundsInRoot()
                val reply = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
                val under = screen.bottom - posture.safe.ime - reply.bottom
                val beside = reply.left - posture.safe.left
                assertTrue(
                    under >= SELECTION_HANDLE,
                    "${posture.name}: $under under the reply box, and a handle hangs " +
                        "$SELECTION_HANDLE below the line it grips",
                )
                assertTrue(
                    beside >= SELECTION_HANDLE,
                    "${posture.name}: $beside beside the reply box, and the start handle hangs " +
                        "$SELECTION_HANDLE to the left of the character it grips",
                )
            }
        }
    }

    // RICH_CONVO ends on a running `Edit` card, and a lazy list only composes what is in view.
    private fun ComposeUiTest.lastTurnIsComposed(): Boolean =
        onAllNodesWithContentDescription("Edit", substring = true).fetchSemanticsNodes().isNotEmpty()
}
