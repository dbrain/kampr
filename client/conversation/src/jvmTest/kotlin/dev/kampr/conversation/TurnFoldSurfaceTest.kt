package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalTextToolbar
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.util.parseIsoMillis
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertTrue

private const val AT = "2026-08-23T09:00:00.000Z"
private val NOW = requireNotNull(parseIsoMillis(AT))

private const val GIST = "Seventy-four columns"
private const val BURIED = "And the scrollbar column"
private const val ASKED = "what did the width inference land on?"
private const val OPEN = "Hide the message of"
private const val SHUT = "Show the message of"
private val AGED = Regex("$SHUT (now|\\d+[mhd]), $GIST\\.")

private val ASK = Turn("u-1", "user", AT, listOf(Block.Md(ASKED)))
private val ANSWER = Turn(
    "a-2", "assistant", AT,
    listOf(Block.Md("$GIST.\n\nIt comes from the pane rect.\n\n$BURIED.")),
)

private fun storeOfAnAnswer(): KamprStore {
    val store = KamprStore()
    store.accept(ServerMsg.Convo(pane = PANE_ID, cursor = "u-1", more = false, turns = listOf(ASK, ANSWER)))
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
class TurnFoldSurfaceTest {
    // Folded means not composed, not merely not painted. That is what keeps a put-away message
    // out of a selection drag as well as off the screen, and it is the whole difference between
    // this and drawing a lid over it.
    @Test
    fun foldingAMessageTakesEveryLineOfItOffTheScreen() = runComposeUiTest {
        setContent { Transcript(storeOfAnAnswer()) }
        waitForIdle()
        onAllNodesWithText(BURIED, substring = true).assertCountEquals(1)

        onNodeWithContentDescription(OPEN, substring = true).performClick()
        waitForIdle()
        onAllNodesWithText(BURIED, substring = true).assertCountEquals(0)
        onAllNodesWithText("It comes from the pane rect", substring = true).assertCountEquals(0)
    }

    // A folded row that says nothing about itself is a worse version of the tool run it sits
    // beside, so it keeps its first line and its age.
    @Test
    fun aFoldedMessageStillSaysWhenItWasSaidAndHowItStarts() = runComposeUiTest {
        setContent { Transcript(storeOfAnAnswer()) }
        waitForIdle()
        onNodeWithContentDescription(OPEN, substring = true).performClick()
        waitForIdle()
        val label = onNodeWithContentDescription(SHUT, substring = true)
            .fetchSemanticsNode().config[SemanticsProperties.ContentDescription].first()
        assertTrue(AGED.matches(label), "the folded row reads \"$label\"")
        onAllNodesWithText("$GIST.", useUnmergedTree = true).assertCountEquals(1)
    }

    @Test
    fun aFoldedMessageIsStillFoldedAfterTheTranscriptTicks() = runComposeUiTest {
        val store = storeOfAnAnswer()
        setContent { Transcript(store) }
        waitForIdle()
        onNodeWithContentDescription(OPEN, substring = true).performClick()
        waitForIdle()
        store.accept(ServerMsg.ConvoTurn(PANE_ID, listOf(proseTurn("a-3", "Anything else?"))))
        waitForIdle()
        onAllNodesWithText(BURIED, substring = true).assertCountEquals(0)
        onNodeWithContentDescription(SHUT, substring = true).assertExists()
    }

    // The same rule the runs got, for the same reason: a match the counter promises and the
    // screen hides is worse than a screen that is too long.
    @Test
    fun aFoldedMessageHoldingWhatTheSearchIsLookingForOpensItself() = runComposeUiTest {
        setContent { Transcript(storeOfAnAnswer()) }
        waitForIdle()
        onNodeWithContentDescription(OPEN, substring = true).performClick()
        waitForIdle()
        onAllNodesWithText(BURIED, substring = true).assertCountEquals(0)

        onNodeWithContentDescription("Search the transcript").performClick()
        onNodeWithContentDescription("Search the transcript").performTextInput("scrollbar")
        waitForIdle()
        onAllNodesWithText(BURIED, substring = true).assertCountEquals(1)
        onNodeWithContentDescription("Match 1 of 1").assertExists()
    }

    // A drag that crosses a folded message must come away with nothing of it — not the line it
    // shows of itself, and not the age beside it. Both are chrome and neither is the message.
    @Test
    fun aDragAcrossAFoldedMessageCopiesNothingOfWhatItHolds() {
        val toolbar = SelectionToolbar()
        val clipboard = HeldClipboard()
        withScene(
            PORTRAIT.first, 300.dp, SoftTheme, TypeScale.Phone,
            content = {
                CompositionLocalProvider(
                    LocalTextToolbar provides toolbar,
                    LocalClipboard provides clipboard,
                ) {
                    SelectionContainer {
                        Column(Modifier.fillMaxSize().padding(16.dp)) {
                            TurnView(ASK, "", emptyList(), {}, Modifier.fillMaxWidth(), now = NOW)
                            TurnView(ANSWER, "", listOf("fold:a-2"), {}, Modifier.fillMaxWidth(), now = NOW)
                        }
                    }
                }
            },
            body = { scene ->
                repeat(3) { scene.render() }
                var at = 0L
                fun touch(kind: PointerEventType, x: Float, y: Float) {
                    scene.sendPointerEvent(kind, Offset(x, y), timeMillis = at, type = PointerType.Touch)
                    at += 50
                }
                touch(PointerEventType.Press, 300f, 66f)
                Thread.sleep(900)
                at = 900
                scene.render()
                touch(PointerEventType.Move, 400f, 140f)
                touch(PointerEventType.Move, 700f, 220f)
                touch(PointerEventType.Release, 700f, 220f)
                scene.render()
                toolbar.copy?.invoke()
                repeat(4) { scene.render(); Thread.sleep(60) }
            },
        )
        val pasted = clipboard.pasted
        assertTrue(pasted != null, "dragging across the folded message produced nothing at all")
        assertTrue("inference" in pasted!!, "nothing above the folded message was copied: $pasted")
        assertTrue(GIST !in pasted, "the folded message's own first line was copied: $pasted")
        assertTrue("now" !in pasted, "the age beside the folded message was copied: $pasted")
    }
}
