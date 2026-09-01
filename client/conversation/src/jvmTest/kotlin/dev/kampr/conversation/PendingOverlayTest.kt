package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val ASKING = "The agent is asking:"
private const val NEWEST_LINE = "This paragraph is the last thing anyone has said in this pane."

private val LONG = ServerMsg.Convo(
    pane = PANE_ID,
    cursor = "l-1",
    more = false,
    turns = (1..12).map { n ->
        Turn(
            id = "l-$n",
            role = if (n % 2 == 0) "assistant" else "user",
            at = null,
            blocks = listOf(
                Block.Md(
                    (1..6).joinToString("\n\n") { "Turn $n, paragraph $it, long enough to take a few lines of a phone." } +
                        if (n == 12) "\n\n$NEWEST_LINE" else "",
                ),
            ),
        )
    },
)

private val ASKED = ServerMsg.Pending(
    pane = PANE_ID,
    question = "Edit crates/kampr-core/src/width.rs?",
    options = listOf(
        PendingOption("1", "Yes, make this edit"),
        PendingOption("2", "Yes, and do not ask again for this file"),
        PendingOption("3", "No, and tell Claude what to do differently"),
    ),
    source = "screen",
)

private val SHORT = ServerMsg.Convo(
    pane = PANE_ID,
    cursor = "s-1",
    more = false,
    turns = listOf(
        Turn("s-1", "user", null, listOf(Block.Md("what did the width inference land on?"))),
        Turn("s-2", "assistant", null, listOf(Block.Md("Seventy-four columns, from the pane's own rect."))),
    ),
)

private fun asked(convo: ServerMsg.Convo?): PaneState {
    val store = KamprStore()
    if (convo == null) store.accept(requireNotNull(Wire.decode(RICH_CONVO)) { "undecodable frame" })
    else store.accept(convo)
    store.accept(ASKED)
    return store.pane(PANE_ID)
}

@OptIn(ExperimentalTestApi::class)
class PendingOverlayTest {
    // The card floats over the top of the scroll region, so whatever the transcript had drawn
    // there was behind it — visible in `conversation-portrait.png` as a line of prose sliced
    // through by the card's top border. Both fixtures, because they are two different ways for
    // the transcript to reach that band: a short one sits at the top of its own scroll range, and
    // a long one is parked on its end and had been scrolling *through* the band all along.
    @Test
    fun anOpenQuestionHasNoTranscriptUnderIt() {
        for ((name, convo) in listOf("a transcript that fits" to SHORT, "a transcript that scrolls" to null)) {
            runComposeUiTest {
                setContent {
                    CompositionLocalProvider(
                        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                        LocalPaneIo provides RecordingIo,
                    ) {
                        Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                            ConversationView(asked(convo), demoInfo(), Modifier.fillMaxSize())
                        }
                    }
                }
                waitForIdle()
                val strip = onNodeWithContentDescription(ASKING, substring = true).fetchSemanticsNode()
                val under = onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.Text))
                    .fetchSemanticsNodes()
                    .filter { !it.isUnder(strip) && it.boundsInRoot.overlapsVisibly(strip.boundsInRoot) }
                assertTrue(
                    under.isEmpty(),
                    "$name: ${under.size} pieces of transcript are drawn inside the question card, " +
                        "which spans ${strip.boundsInRoot}: ${under.map { it.boundsInRoot }}",
                )
            }
        }
    }

    // The report: the card is drawn, its chips are drawn, and pressing one sends nothing. The
    // strip on its own has always answered (`AccessibilityTest`), so the tap has to be taken
    // through the view that actually carries it.
    @Test
    fun pressingAnAnswerOnTheOpenQuestionSendsIt() = runComposeUiTest {
        RecordingIo.sent.clear()
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
                LocalConnectionStatus provides ConnectionStatus.Live("full"),
            ) {
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(asked(SHORT), demoInfo(), Modifier.fillMaxSize())
                }
            }
        }
        waitForIdle()
        onNodeWithContentDescription("Answer 2, Yes, and do not ask again for this file").performClick()
        waitForIdle()
        assertEquals(
            listOf(ClientMsg.Answer(PANE_ID, "2")),
            RecordingIo.sent.toList(),
            "the chip was pressed and the answer never left the client",
        )
    }

    // The report: *"we show blocked with options when Claude asks a question, but tapping the
    // options do nothing"*, on both the card and the terminal's chip row, and *"it worked slowly
    // after a couple of presses"*. An answer is `typing` on the wire (`KamprConnection.typing`),
    // so it is dropped the moment the socket is not live — and the phone is exactly there when it
    // is opened on a blocked-agent notification: the card is drawn from memory over a connection
    // still climbing its backoff ladder. A chip that presses and delivers nothing is the row of
    // keys this codebase has paid for twice.
    @Test
    fun anAnswerThatCannotLeaveTheDeviceIsNotOfferedAsAPressableChip() = runComposeUiTest {
        RecordingIo.sent.clear()
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
                LocalConnectionStatus provides ConnectionStatus.Offline("the node stopped answering", 4_000),
            ) {
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(asked(SHORT), demoInfo(), Modifier.fillMaxSize())
                }
            }
        }
        waitForIdle()
        onNodeWithContentDescription("Answer 1, Yes, make this edit").assertIsNotEnabled()
        assertTrue(
            onAllNodesWithText("not connected", substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the card offered three answers over a dead socket and said nothing about it",
        )
    }

    // The other half: the socket was live when the chip was pressed and died with the frame still
    // in the queue, so `discardTyping` counted it lost. The card is the only place that press was
    // made and the only place its loss can be reported.
    @Test
    fun anAnswerTheSocketLostIsReportedOnTheCardThatSentIt() = runComposeUiTest {
        RecordingIo.sent.clear()
        val pane = asked(SHORT)
        pane.noteUndelivered()
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
                LocalConnectionStatus provides ConnectionStatus.Live("full"),
            ) {
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(pane, demoInfo(), Modifier.fillMaxSize())
                }
            }
        }
        waitForIdle()
        assertTrue(
            onAllNodesWithText("did not get through", substring = true).fetchSemanticsNodes().isNotEmpty(),
            "an answer that never left the device was silently forgotten",
        )
    }

    // The band the card is given comes out of the top of the list, and a lazy list anchors on its
    // first visible item — so a question arriving on a reader who is standing on the end of the
    // transcript pushes that end down by the whole height of the card and leaves it there.
    @Test
    fun aQuestionArrivingDoesNotPushTheEndOfTheTranscriptOffTheFold() = runComposeUiTest {
        val store = KamprStore()
        store.accept(LONG)
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
            ) {
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(store.pane(PANE_ID), demoInfo(), Modifier.fillMaxSize())
                }
            }
        }
        waitForIdle()
        val settled = endOfTheTranscript()
        store.accept(ASKED)
        waitForIdle()
        val end = endOfTheTranscript()
        assertTrue(end <= settled + 1.dp, "the question moved the end of the transcript from $settled to $end")
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.endOfTheTranscript(): Dp {
    assertTrue(
        onAllNodesWithText(NEWEST_LINE, substring = true).fetchSemanticsNodes().isNotEmpty(),
        "the end of the newest turn was never composed at all",
    )
    return onNodeWithText(NEWEST_LINE, substring = true).getUnclippedBoundsInRoot().bottom
}

// Clipped bounds, so a turn scrolled behind the card reports the sliver of itself that is still
// painted. A node clipped away entirely comes back with no area rather than absent.
private fun Rect.overlapsVisibly(other: Rect): Boolean =
    width > 0.5f && height > 0.5f && overlaps(other)

// The card's own answer chips merge their labels and so match a search for text as readily as a
// turn does — they are the card, not something under it.
private fun SemanticsNode.isUnder(other: SemanticsNode): Boolean {
    var node: SemanticsNode? = this
    while (node != null) {
        if (node.id == other.id) return true
        node = node.parent
    }
    return false
}
