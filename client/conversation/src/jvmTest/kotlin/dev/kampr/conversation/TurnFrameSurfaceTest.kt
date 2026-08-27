package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
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
    // width and read as a different kind of thing from the answer under it. Both are turns, and
    // both get the whole column; what tells them apart is the colour of the card, not its inset.
    @Test
    fun aReplyGetsTheSameColumnTheAnswerDoes() = runComposeUiTest {
        setContent { Transcript(bothSpeakers()) }
        waitForIdle()
        val reply = onNodeWithText(ASKED, substring = true).fetchSemanticsNode().boundsInRoot
        val answer = onNodeWithText(ANSWERED, substring = true).fetchSemanticsNode().boundsInRoot
        assertEquals(answer.left, reply.left, "a reply starts at a different column from an answer")
        // Both wrap, so both reach the column's own right edge — a bubble that hugged its text
        // would fall short of it by whatever the last line did not use.
        assertEquals(answer.right, reply.right, "a reply ends at a different column from an answer")
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
