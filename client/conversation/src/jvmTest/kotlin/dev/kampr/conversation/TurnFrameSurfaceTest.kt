package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.BrutalistFamily
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.PhosphorFamily
import dev.kampr.shared.theme.SoftFamily
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.WarmFamily
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.on
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

private const val ASKED =
    "which keys does the grammar accept, and does it take the ones the review strip already draws?"
private const val ANSWERED =
    "Six, and they are documented in the probe log beside the measurement that established each one."

private fun bothSpeakers(): KamprStore {
    val store = KamprStore()
    store.accept(
        ServerMsg.Convo(
            pane = PANE_ID, cursor = "u-1", more = false,
            turns = listOf(
                proseTurn("u-1", ASKED, role = "user"),
                proseTurn("a-2", ANSWERED),
            ),
        ),
    )
    return store
}

@Composable
private fun Transcript(store: KamprStore) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
    ) {
        Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
            ConversationView(store.pane(PANE_ID), demoInfo(), Modifier.fillMaxSize())
        }
    }
}

@OptIn(ExperimentalTestApi::class)
class TurnFrameSurfaceTest {
    // A reply used to hug its own text in a gutter on the right, which cost it 44 dp of a phone's
    // width and read as a different kind of thing from the answer under it. What the reader gets
    // instead is one box per block: the head of an answer and every step of it share a column and
    // a ground, and what separates one block from the next is the gap between the boxes.
    //
    // Bounds and not pixels of paint, because the box is drawn a piece at a time and no one piece
    // holds it — `ExchangeTest` is where the pieces are proved to agree on which box they are in.
    @Test
    fun anAskAndAWholeReplyEachGetOneColumnOfTheirOwn() = runComposeUiTest {
        setContent { Transcript(bothSpeakers()) }
        waitForIdle()
        val ask = onNodeWithText(ASKED, substring = true).fetchSemanticsNode().boundsInRoot
        val head = onNodeWithContentDescription("Put away the reply of", substring = true)
            .fetchSemanticsNode().boundsInRoot
        val step = onNodeWithText(ANSWERED, substring = true).fetchSemanticsNode().boundsInRoot
        assertEquals(ask.left, head.left, "an ask and a reply head start at different columns")
        assertEquals(head.left, step.left, "a step is indented out of the box its head draws")
        assertEquals(ask.right, head.right, "an ask and a reply head end at different columns")
    }

    // The trap this walked into twice: `accent` and `working` are the *same colour* in Phosphor and
    // in Warm, and `done` is `text` in Brutalist, so a scheme built on two status hues reads as one
    // in half the themes shipped. Every theme, both grounds, or the frame says nothing.
    @Test
    fun theTwoSpeakersWearDifferentColoursInEveryThemeOnBothGrounds() {
        for (family in listOf(SoftFamily, PhosphorFamily, WarmFamily, BrutalistFamily)) {
            for (ground in Ground.entries) {
                val tokens = tokensFor(family.on(ground), TypeScale.Phone, ground)
                val you = speakerSkin(tokens, Speaker.You, "claude")
                val agent = speakerSkin(tokens, Speaker.Agent, "claude")
                val where = "${family.id.key} on $ground"
                assertNotEquals(you.rail, agent.rail, "one rail colour for both speakers in $where")
                assertNotEquals(you.label, agent.label, "one label for both speakers in $where")
                assertTrue(you.rail.alpha > 0f && agent.rail.alpha > 0f, "an invisible rail in $where")
            }
        }
    }
}
