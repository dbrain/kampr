package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import androidx.compose.foundation.layout.size
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertTrue

private const val NEWEST_LINE = "This paragraph is the last thing anyone has said in this pane."
private const val TO_END = "Go to the end of the transcript"
private const val OLDEST_LINE = "An older page, fetched after the transcript was already open."

private fun body(turn: Int) =
    (1..6).joinToString("\n\n") { "Turn $turn, paragraph $it, long enough to take a few lines of a phone." }

private fun transcript(turns: Int, more: Boolean) = ServerMsg.Convo(
    pane = PANE_ID,
    cursor = "c-1",
    more = more,
    turns = (1..turns).map { n ->
        Turn(
            id = "t-$n",
            role = if (n % 2 == 0) "assistant" else "user",
            at = "2026-08-23T09:00:00.000Z",
            blocks = listOf(Block.Md(body(n) + (if (n == turns) "\n\n$NEWEST_LINE" else ""))),
        )
    },
)

private fun olderPage() = ServerMsg.Convo(
    pane = PANE_ID,
    cursor = null,
    more = false,
    turns = listOf(Turn("t-0", "user", "2026-08-23T08:59:00.000Z", listOf(Block.Md(OLDEST_LINE)))),
)

// The whole pane, with the switch the reader actually presses. A tab switch is not a scroll and
// not a recomposition: `PaneScreenMobile` swaps the surface outright, so the conversation — its
// lazy list state with it — is disposed and built again from nothing.
@Composable
private fun WholePane(store: KamprStore, view: PaneView) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
    ) {
        Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
            PaneScreenMobile(
                pane = store.pane(PANE_ID),
                info = demoInfo(),
                view = view,
                surfaces = ConversationSurfaces(),
                landscape = false,
                readOnly = false,
                onBack = {},
                onView = {},
                onAnswer = {},
            )
        }
    }
}

// PaneScreenMobile lays the transcript out under a *guessed* chrome height — 108 dp in portrait,
// 44 dp in landscape — and replaces the guess with the header's own height the moment
// `onGloballyPositioned` reports it, which is after the transcript's first layout.
@Composable
private fun UnderChrome(store: KamprStore, guess: Dp, real: Dp) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
    ) {
        var chrome by remember { mutableStateOf<Dp?>(null) }
        val density = LocalDensity.current
        Box(Modifier.fillMaxSize()) {
            ConversationView(
                store.pane(PANE_ID),
                demoInfo(),
                Modifier.fillMaxSize().padding(top = chrome ?: guess),
            )
            Box(
                Modifier
                    .align(Alignment.TopStart)
                    .fillMaxWidth()
                    .height(real)
                    .background(Color.Red)
                    .onGloballyPositioned { chrome = with(density) { it.size.height.toDp() } },
            )
        }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.endOfTheTranscript(): Dp {
    val nodes = onAllNodesWithText(NEWEST_LINE, substring = true).fetchSemanticsNodes()
    assertTrue(nodes.isNotEmpty(), "the end of the newest turn was never composed at all")
    return onNodeWithText(NEWEST_LINE, substring = true).getUnclippedBoundsInRoot().bottom
}

@OptIn(ExperimentalTestApi::class)
class TranscriptFollowTest {
    // The measurement this whole test class exists for: where the end of the transcript lands
    // when the chrome above it never moves. Everything else has to match it.
    private fun settledEnd(chrome: Dp): Dp {
        var end = 0.dp
        runComposeUiTest {
            val store = KamprStore()
            store.accept(transcript(turns = 12, more = false))
            setContent { UnderChrome(store, guess = chrome, real = chrome) }
            waitForIdle()
            end = endOfTheTranscript()
        }
        return end
    }

    // Reported from a phone, and it had survived one fix already: "the conversation pane has
    // consistently struggled to scroll to the bottom when selected". Every previous attempt aimed
    // at the scroll *target*, which was never wrong. The viewport is.
    //
    // A lazy list anchors on its first visible item, so when the pane is re-laid out shorter —
    // which is exactly what the corrected chrome height does, once, on every single open — the
    // end of the transcript slides down by however much the guess was short by and the reader is
    // left above it. Nothing in the turn data changed, so nothing re-aimed the scroll.
    @Test
    fun aChromeHeightThatArrivesAfterTheFirstLayoutLeavesTheReaderAtTheEnd() = runComposeUiTest {
        val settled = settledEnd(160.dp)
        val store = KamprStore()
        store.accept(transcript(turns = 12, more = false))
        setContent { UnderChrome(store, guess = 108.dp, real = 160.dp) }
        waitForIdle()
        val end = endOfTheTranscript()
        assertTrue(
            end <= settled + 1.dp,
            "the transcript ends at $end, $end minus $settled below where a settled pane puts it",
        )
    }

    // The same defect from the other end: a turn that grows after it was laid out. `Block.Code` is
    // the ordinary way an answer ends, it arrives as a revision of a turn already on screen, and
    // it changes no count of turns and no count of prose characters — so a scroll that re-aims
    // itself on either of those numbers cannot see it.
    @Test
    fun aTurnThatGrowsWithoutGainingProseKeepsTheReaderAtTheEnd() = runComposeUiTest {
        val settled = settledEnd(120.dp)
        val store = KamprStore()
        store.accept(transcript(turns = 12, more = false))
        setContent { UnderChrome(store, guess = 120.dp, real = 120.dp) }
        waitForIdle()
        store.accept(
            ServerMsg.ConvoTurn(
                pane = PANE_ID,
                turns = listOf(
                    Turn(
                        "t-12", "assistant", "2026-08-23T09:00:09.000Z",
                        listOf(
                            Block.Code("kotlin", (1..12).joinToString("\n") { "    val line$it = $it" }),
                            Block.Md(body(12) + "\n\n" + NEWEST_LINE),
                        ),
                    ),
                ),
            ),
        )
        waitForIdle()
        val end = endOfTheTranscript()
        assertTrue(end <= settled + 1.dp, "the revised turn ended at $end, below the settled $settled")
    }

    // The transcript does not exist yet when the pane is opened: `convo` arrives over the socket
    // after the view is composed, and the page before it after that.
    @Test
    fun aTranscriptThatArrivesAfterTheOpenLandsOnItsEndAndStaysThere() = runComposeUiTest {
        val settled = settledEnd(120.dp)
        val store = KamprStore()
        store.pane(PANE_ID)
        setContent { UnderChrome(store, guess = 120.dp, real = 120.dp) }
        waitForIdle()
        store.accept(transcript(turns = 12, more = true))
        waitForIdle()
        assertTrue(endOfTheTranscript() <= settled + 1.dp, "the arriving transcript did not land on its end")
        store.accept(olderPage())
        waitForIdle()
        assertTrue(endOfTheTranscript() <= settled + 1.dp, "the older page pulled the reader off the end")
        assertTrue(
            onAllNodesWithText(OLDEST_LINE, substring = true).fetchSemanticsNodes().isEmpty(),
            "the older page put the top of the transcript on screen",
        )
    }

    // Reported from a phone against 0.1.17: "switching between terminal and conversation the
    // conversation pane starts from the top every time". A tab switch disposes the conversation
    // outright, so coming back is a first open onto a transcript that is already long — and the
    // one thing a return has that a first open does not is a chrome height that is already
    // correct, so nothing re-lays the list out and nothing fires a second time.
    @Test
    fun aReturnFromTheTerminalLandsOnTheEndOfTheTranscriptRatherThanItsTop() = runComposeUiTest {
        val store = KamprStore()
        store.accept(transcript(turns = 12, more = false))
        var view by mutableStateOf(PaneView.Conversation)
        setContent { WholePane(store, view) }
        waitForIdle()
        val opened = endOfTheTranscript()

        view = PaneView.Terminal
        waitForIdle()
        assertTrue(
            onAllNodesWithText(NEWEST_LINE, substring = true).fetchSemanticsNodes().isEmpty(),
            "the conversation was still composed while the terminal was showing",
        )
        view = PaneView.Conversation
        waitForIdle()
        assertTrue(
            endOfTheTranscript() <= opened + 1.dp,
            "the return landed at ${endOfTheTranscript()}, below the $opened the first open found",
        )
        assertTrue(
            onAllNodesWithText("Turn 1, paragraph 1", substring = true).fetchSemanticsNodes().isEmpty(),
            "the return put the top of the transcript on screen",
        )
    }

    // The manual way back, and it is only offered when it would do something: a reader standing
    // on the end has nowhere to go, and a glyph that is always there costs the rotated bar a
    // target for nothing. `following` is the same signal that decides whether the transcript
    // chases its own end, so there is one notion of "away from the end" and not two.
    @Test
    fun theWayBackToTheEndIsOfferedOnlyOnceTheReaderHasLeftIt() = runComposeUiTest {
        val store = KamprStore()
        store.accept(transcript(turns = 12, more = false))
        // The newest turn is deliberately taller than the viewport: aiming a lazy list at an
        // item aims at its *top*, and on a turn that fits, the top and the end are the same
        // place — which is a harness that cannot see the defect it was written for.
        val tall = (1..20).joinToString("\n\n") { "Turn 12, paragraph $it, long enough to take a few lines of a phone." }
        store.accept(
            ServerMsg.ConvoTurn(
                pane = PANE_ID,
                turns = listOf(
                    Turn("t-12", "assistant", "2026-08-23T09:00:00.000Z", listOf(Block.Md("$tall\n\n$NEWEST_LINE"))),
                ),
            ),
        )
        setContent { UnderChrome(store, guess = 120.dp, real = 120.dp) }
        waitForIdle()
        val settled = endOfTheTranscript()
        onAllNodesWithContentDescription(TO_END).assertCountEquals(0)

        scrollBack()
        onNodeWithContentDescription(TO_END).assertExists()
        assertTrue(
            onAllNodesWithText(NEWEST_LINE, substring = true).fetchSemanticsNodes().isEmpty(),
            "the drag did not actually leave the end of the transcript",
        )

        onNodeWithContentDescription(TO_END).performClick()
        waitForIdle()
        assertTrue(
            endOfTheTranscript() <= settled + 1.dp,
            "the way back landed at ${endOfTheTranscript()} rather than the $settled it started at",
        )
        onAllNodesWithContentDescription(TO_END).assertCountEquals(0)
    }

    private fun ComposeUiTest.scrollBack() {
        repeat(2) {
            onNode(hasScrollAction()).performTouchInput {
                down(centerLeft + Offset(4f, 0f))
                repeat(8) { moveBy(Offset(0f, 60f)) }
                up()
            }
            waitForIdle()
        }
    }

    // The other half of the contract, and the one that a scroll which re-aims itself on every
    // layout would break: a reader who has scrolled back to read something stays where they put
    // themselves, however much the agent writes underneath them.
    @Test
    fun aReaderWhoScrolledBackIsLeftWhereTheyPutThemselves() = runComposeUiTest {
        val store = KamprStore()
        store.accept(transcript(turns = 12, more = false))
        setContent { UnderChrome(store, guess = 120.dp, real = 120.dp) }
        waitForIdle()
        scrollBack()
        val readingHere = onNodeWithText("Turn 2, paragraph 1", substring = true)
            .getUnclippedBoundsInRoot().top
        store.accept(
            ServerMsg.ConvoTurn(
                pane = PANE_ID,
                turns = listOf(
                    Turn("t-13", "assistant", "2026-08-23T09:00:20.000Z", listOf(Block.Md(body(13)))),
                ),
            ),
        )
        waitForIdle()
        val stillHere = onNodeWithText("Turn 2, paragraph 1", substring = true)
            .getUnclippedBoundsInRoot().top
        assertTrue(
            (stillHere - readingHere).value in -1f..1f,
            "the reader was dragged from $readingHere to $stillHere by a turn arriving below them",
        )
    }
}
