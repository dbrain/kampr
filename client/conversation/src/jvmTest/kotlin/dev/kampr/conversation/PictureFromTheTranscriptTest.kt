package dev.kampr.conversation

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.waitUntilAtLeastOneExists
import dev.kampr.conversation.files.Stack
import dev.kampr.conversation.files.Stage
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.ui.recordPlace
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val SAID = "Here is what the pane looked like."
private const val VIEWER = "shot.png, pinch to zoom, double tap to fit"

private fun paneWithAPicture(): PaneState {
    val store = KamprStore()
    store.accept(
        ServerMsg.Convo(
            pane = PANE_ID,
            cursor = "a-1",
            more = false,
            turns = listOf(
                Turn("u-0", "user", "2026-08-24T09:00:00.000Z", listOf(Block.Md("show me"))),
                Turn("a-1", "assistant", "2026-08-24T09:00:06.000Z", listOf(Block.Md(SAID), Block.Md(MARKER, SHOT))),
            ),
        ),
    )
    return store.pane(PANE_ID)
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.openTheShot(): Pair<NodeWithAttachments, Stack> {
    val node = NodeWithAttachments(mapOf(SHOT.id to AttachmentBytes.Ok(pngBytes(900, 1400), "image/png")))
    val nav = Stack()
    setContent { Stage(node, nav) { ConversationView(paneWithAPicture(), demoInfo(), Modifier.fillMaxSize()) } }
    waitForIdle()
    onNodeWithContentDescription("Show image, shot.png").performClick()
    waitUntil(timeoutMillis = 5_000) {
        onAllNodesWithContentDescription("Open shot.png").fetchSemanticsNodes().isNotEmpty()
    }
    onNodeWithContentDescription("Open shot.png").performClick()
    waitUntilAtLeastOneExists(hasContentDescription(VIEWER), 5_000)
    return node to nav
}

// The picture used to open inside the conversation, with a back handler of its own racing the
// app's — so the phone's back button left the pane instead of the picture. It opens on the app's
// files stack now, which `SystemBack` pops before anything else.
@OptIn(ExperimentalTestApi::class)
class PictureFromTheTranscriptTest {
    @Test
    fun aThumbnailOpensThePictureOnTheFilesStackOverTheWholeWindow() = runComposeUiTest {
        val (_, nav) = openTheShot()
        assertEquals(listOf(recordPlace(PANE_ID, SHOT)), nav.places.toList())
        onNodeWithContentDescription(VIEWER).assertIsDisplayed()

        onNodeWithContentDescription("Back from shot.png").performClick()
        waitForIdle()
        assertTrue(nav.places.isEmpty())
        assertTrue(
            onAllNodesWithText(SAID, substring = true).fetchSemanticsNodes().isNotEmpty(),
            "closing the picture did not give the transcript back",
        )
    }

    // The bytes are already in hand from the thumbnail, so the viewer and its Download use them
    // rather than a second authorised fetch of 730 KB over a phone link.
    @Test
    fun openingThePictureDoesNotFetchItASecondTime() = runComposeUiTest {
        val (node, _) = openTheShot()
        onNodeWithContentDescription("Download shot.png to this device").assertIsDisplayed()
        assertEquals(1, node.asked.size, "opening the picture fetched it a second time: ${node.asked}")
    }
}
