package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlinx.coroutines.CompletableDeferred
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertTrue

private const val PICTURE = "Open shot.png"

private const val ANCHOR_LINE = "The last thing the agent said before the screenshot went up."

private fun tallAnswerEndingOnTheAnchor(): String =
    (1..40).joinToString("\n\n") { "Paragraph $it of an answer that runs well past one screen." } +
        "\n\n" + ANCHOR_LINE

private fun paneEndingOnAnAttachment(): PaneState {
    val store = KamprStore()
    store.accept(
        ServerMsg.Convo(
            pane = PANE_ID,
            cursor = "a-1",
            more = false,
            turns = listOf(
                Turn("a-1", "assistant", "2026-08-24T09:00:00.000Z", listOf(Block.Md(tallAnswerEndingOnTheAnchor()))),
                Turn("u-2", "user", "2026-08-24T09:00:06.000Z", listOf(Block.Md(MARKER, SHOT))),
            ),
        )
    )
    return store.pane(PANE_ID)
}

@OptIn(ExperimentalTestApi::class)
class AttachmentScrollTest {
    // A picture arriving is not news the list should chase. The transcript already has one defect
    // to its name for aiming a scroll at the wrong thing when an item's height changed under the
    // reader (see TranscriptEndTest), and an image is the largest height change this view has.
    @Test
    fun anImageArrivingDoesNotMoveTheTranscriptUnderTheReader() = runComposeUiTest {
        val pane = paneEndingOnAnAttachment()
        val gate = CompletableDeferred<Unit>()
        val node = NodeWithAttachments(
            mapOf(SHOT.id to AttachmentBytes.Ok(pngBytes(900, 1400), "image/png")),
            gate,
        )
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides node,
            ) {
                Box(Modifier.fillMaxSize()) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) }
            }
        }
        waitForIdle()
        onNodeWithContentDescription("Show image, shot.png").performClick()
        // Measured with the node still holding the bytes: pressing a control near the bottom edge
        // brings it into view on its own, which is every button in this view and not the thing
        // under test. What is under test is the picture landing afterwards.
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription("Fetching shot.png").fetchSemanticsNodes().isNotEmpty()
        }
        val before = onNodeWithText(ANCHOR_LINE, substring = true).getUnclippedBoundsInRoot().top

        gate.complete(Unit)
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription(PICTURE).fetchSemanticsNodes().isNotEmpty()
        }
        waitForIdle()

        assertTrue(
            onAllNodesWithText(ANCHOR_LINE, substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the line the reader was looking at was scrolled off the screen by the picture arriving",
        )
        val after = onNodeWithText(ANCHOR_LINE, substring = true).getUnclippedBoundsInRoot().top
        assertTrue(
            abs((after - before).value) < 1f,
            "the transcript moved under the reader: $before became $after",
        )
    }

    // The other half of the same rule, and it used to be arithmetic: the picture was drawn at the
    // column's width and clamped to half a phone, so a 900x1400 screenshot had to be measured
    // rather than declared or it ran 672 dp down a column it was supposed to fit inside. What
    // lands now is a thumbnail of a fixed size, so the bound is the size itself — but the rule it
    // enforces is the same one and this is where it is written down.
    @Test
    fun aTallScreenshotIsBoundedByTheColumnRatherThanTheOtherWayAround() = runComposeUiTest {
        val pane = paneEndingOnAnAttachment()
        val node = NodeWithAttachments(mapOf(SHOT.id to AttachmentBytes.Ok(pngBytes(900, 1400), "image/png")))
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides node,
            ) {
                Box(Modifier.fillMaxSize()) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) }
            }
        }
        waitForIdle()
        onNodeWithContentDescription("Show image, shot.png").performClick()
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription(PICTURE).fetchSemanticsNodes().isNotEmpty()
        }
        val screen = onRoot().getUnclippedBoundsInRoot()
        val card = onNodeWithContentDescription(PICTURE).getUnclippedBoundsInRoot()
        val wide = card.right - card.left
        val tall = card.bottom - card.top
        assertTrue(wide <= screen.right - screen.left, "the card is $wide wide in ${screen.right - screen.left}")
        assertTrue(tall <= 120.dp, "a 900x1400 screenshot took $tall of the column")
    }
}
