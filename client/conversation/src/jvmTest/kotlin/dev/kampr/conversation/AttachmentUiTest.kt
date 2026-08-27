package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.Attachment
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Turn
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import javax.imageio.ImageIO
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

internal const val MARKER = "[image · png]"

internal val SHOT = Attachment(
    id = "att-7f3",
    kind = "image",
    mime = "image/png",
    bytes = 52831,
    name = "shot.png",
)

internal val ARCHIVE = Attachment(
    id = "att-zip",
    kind = "archive",
    mime = "application/zip",
    bytes = 1_400_000,
    name = "logs.zip",
)

internal fun pngBytes(width: Int, height: Int): ByteArray {
    val image = BufferedImage(width, height, BufferedImage.TYPE_INT_RGB)
    for (y in 0 until height) for (x in 0 until width) image.setRGB(x, y, (x * 7 + y * 13) and 0xFFFFFF)
    val out = ByteArrayOutputStream()
    ImageIO.write(image, "png", out)
    return out.toByteArray()
}

// A node that answers exactly what it was asked for, and records the question: the route and the
// pane are the two things a card can get wrong without anything on screen looking different.
internal class NodeWithAttachments(
    private val answers: Map<String, AttachmentBytes>,
    // A node that has not answered yet, so a test can look at the card while it is still fetching.
    private val held: CompletableDeferred<Unit>? = null,
) : PaneIo {
    val asked = mutableListOf<Pair<String, String>>()

    override fun send(msg: ClientMsg) = Unit

    override fun prefs(paneId: String): PanePrefs = PanePrefs()

    override suspend fun attachment(paneId: String, id: String): AttachmentBytes {
        asked += paneId to id
        held?.await()
        return answers[id] ?: AttachmentBytes.Failed("The node no longer has this attachment.")
    }
}

private fun attachedTurn(att: Attachment, id: String = "u-1") =
    Turn(id, "user", "2026-08-24T09:00:01.000Z", listOf(Block.Md(MARKER, att)))

@Composable
private fun Card(turn: Turn, io: PaneIo, store: AttachmentStore) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides io,
    ) {
        Box(Modifier.fillMaxSize()) {
            TurnView(turn, "", emptyList(), {}, Modifier.fillMaxWidth(), store)
        }
    }
}

@OptIn(ExperimentalTestApi::class)
class AttachmentUiTest {
    private val silent = NodeWithAttachments(emptyMap())

    // The marker is what a client that cannot fetch shows. This one can, so the marker is a card
    // with one press on it — and the bracketed text never reaches the reader.
    @Test
    fun aBlockCarryingAHeaderPressesRatherThanPrintingItsMarker() = runComposeUiTest {
        setContent { Card(attachedTurn(SHOT), silent, AttachmentStore(PANE_ID)) }
        onNodeWithContentDescription("Show image, shot.png").assertHeightIsAtLeast(44.dp)
        onNodeWithText("shot.png").assertExists()
        onNodeWithText("png · 52.8 KB").assertExists()
        assertTrue(
            onAllNodesWithText(MARKER, substring = true).fetchSemanticsNodes().isEmpty(),
            "the raw marker was rendered beside the card",
        )
    }

    @Test
    fun aBlockWithNoHeaderIsTheProseItAlwaysWas() = runComposeUiTest {
        val turn = Turn("a-1", "assistant", null, listOf(Block.Md("nothing attached here")))
        setContent { Card(turn, silent, AttachmentStore(PANE_ID)) }
        onNodeWithText("nothing attached here", substring = true).assertExists()
        assertTrue(
            onAllNodesWithContentDescription("Show image", substring = true).fetchSemanticsNodes().isEmpty(),
            "a plain paragraph grew a fetch button",
        )
    }

    // The additive rule, on screen: a kind with no viewer in this release is a download, never a
    // block that quietly vanishes out of the transcript.
    @Test
    fun aKindThisClientDoesNotKnowOffersADownloadRatherThanVanishing() = runComposeUiTest {
        setContent { Card(attachedTurn(ARCHIVE), silent, AttachmentStore(PANE_ID)) }
        onNodeWithContentDescription("Download file, logs.zip").assertHeightIsAtLeast(44.dp)
        onNodeWithText("logs.zip").assertExists()
    }

    @Test
    fun pressingShowImageAsksThatPaneForThatIdAndShowsWhatComesBack() = runComposeUiTest {
        val node = NodeWithAttachments(mapOf(SHOT.id to AttachmentBytes.Ok(pngBytes(320, 200), "image/png")))
        setContent { Card(attachedTurn(SHOT), node, AttachmentStore(PANE_ID)) }
        onNodeWithContentDescription("Show image, shot.png").performClick()
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription("Open shot.png").fetchSemanticsNodes().isNotEmpty()
        }
        assertEquals(listOf(PANE_ID to SHOT.id), node.asked)
    }

    // A blank box where a picture should be is the defect this project cares most about: a fetch
    // that failed says so, in the node's own words, and offers the press again.
    @Test
    fun aFetchThatFailsSaysWhyAndOffersToTryAgain() = runComposeUiTest {
        val node = NodeWithAttachments(mapOf(SHOT.id to AttachmentBytes.Failed("The node no longer has this attachment.")))
        setContent { Card(attachedTurn(SHOT), node, AttachmentStore(PANE_ID)) }
        onNodeWithContentDescription("Show image, shot.png").performClick()
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithText("The node no longer has this attachment.", substring = true)
                .fetchSemanticsNodes().isNotEmpty()
        }
        onNodeWithContentDescription("Try again, shot.png").performClick()
        waitUntil(timeoutMillis = 5_000) { node.asked.size == 2 }
        assertEquals(listOf(PANE_ID to SHOT.id, PANE_ID to SHOT.id), node.asked)
    }

    // A card that is fetching says it is fetching. The alternative is a press that does nothing
    // visible for as long as the link takes, which reads as a broken button.
    @Test
    fun aCardThatIsFetchingSaysSoUntilTheBytesLand() = runComposeUiTest {
        val gate = CompletableDeferred<Unit>()
        val node = NodeWithAttachments(
            mapOf(SHOT.id to AttachmentBytes.Ok(pngBytes(320, 200), "image/png")),
            gate,
        )
        setContent { Card(attachedTurn(SHOT), node, AttachmentStore(PANE_ID)) }
        onNodeWithContentDescription("Show image, shot.png").performClick()
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription("Fetching shot.png").fetchSemanticsNodes().isNotEmpty()
        }
        gate.complete(Unit)
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription("Open shot.png").fetchSemanticsNodes().isNotEmpty()
        }
    }

    // Bytes that are not a picture are a failure with a reason, not a card that sits on "fetching"
    // for ever and not an empty frame.
    @Test
    fun bytesThatAreNotAPictureAreReportedRatherThanShownAsNothing() = runComposeUiTest {
        val node = NodeWithAttachments(
            mapOf(SHOT.id to AttachmentBytes.Ok("this is not a png".encodeToByteArray(), "image/png")),
        )
        setContent { Card(attachedTurn(SHOT), node, AttachmentStore(PANE_ID)) }
        onNodeWithContentDescription("Show image, shot.png").performClick()
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithText("not a picture", substring = true).fetchSemanticsNodes().isNotEmpty()
        }
        onNodeWithContentDescription("Try again, shot.png").assertExists()
    }
}

// Decoded pixels, not the bytes off the wire: a 730 KB screenshot is ~9 MB of them, and a
// transcript can mention a day's worth.
class AttachmentBoundTest {
    @Test
    fun theStoreLetsGoOfTheOldestPictureWhenItIsHoldingTooMany() {
        val shots = (1..6).map { Attachment(id = "att-$it", kind = "image", mime = "image/png") }
        val node = NodeWithAttachments(
            shots.associate { it.id to AttachmentBytes.Ok(pngBytes(64, 64), "image/png") },
        )
        val store = AttachmentStore(PANE_ID, mostImagesHeld = 3, mostPixelBytesHeld = 64L * 1024 * 1024)
        runBlocking { for (shot in shots) store.open(node, shot) }

        assertEquals(
            listOf("att-4", "att-5", "att-6"),
            shots.map { it.id }.filter { store.state(it) is AttachmentState.Shown },
            "the three most recently opened pictures are the three still held",
        )
        assertEquals(AttachmentState.Idle, store.state("att-1"), "an evicted picture is a button again")
    }

    @Test
    fun onePictureBiggerThanTheWholeBudgetIsStillShown() {
        val shot = Attachment(id = "att-huge", kind = "image", mime = "image/png")
        val node = NodeWithAttachments(mapOf(shot.id to AttachmentBytes.Ok(pngBytes(256, 256), "image/png")))
        val store = AttachmentStore(PANE_ID, mostImagesHeld = 4, mostPixelBytesHeld = 1)
        runBlocking { store.open(node, shot) }
        assertTrue(store.state(shot.id) is AttachmentState.Shown)
    }
}
