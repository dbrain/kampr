package dev.kampr.conversation.files

import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.test.waitUntilAtLeastOneExists
import dev.kampr.conversation.Decoder
import dev.kampr.conversation.Projector
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.filePlace
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "%7"
private const val CLIP = "/home/u/demo/clip.wav"
private const val FILM = "/home/u/demo/film.mp4"

// Every ticket is new, as the node's are, and the first `refusals` asks are turned away, as they
// are while a node restarts.
private class Ticketing(answers: Map<String, AttachmentBytes>, private var refusals: Int = 0) : PaneIo by FileNode(answers) {
    val minted = mutableListOf<String>()

    override suspend fun streamUrl(paneId: String, id: String): String? {
        if (refusals > 0) {
            refusals--
            return null
        }
        return "${streamOf(id)}?n=${minted.size}".also { minted += it }
    }
}

private fun clip() = mapOf(fileAttachmentId(CLIP) to AttachmentBytes.Ok(ByteArray(64) { it.toByte() }, "audio/wav"))

private fun film() = mapOf(fileAttachmentId(FILM) to AttachmentBytes.Ok(ByteArray(64) { it.toByte() }, "video/mp4"))

// A ticket dies with the node that minted it and after twelve hours. A player left holding a dead
// one asks for another, and a recording picks up where it stopped.
@OptIn(ExperimentalTestApi::class)
class StreamAgainTest {
    @Test
    fun aRecordingWhoseStreamDroppedPlaysOnFromWhereItWasOnANewTicket() = runComposeUiTest {
        val node = Ticketing(clip())
        val decoder = Decoder(lasts = 10 * 60_000L)
        setContent { Stage(node, Stack(filePlace(PANE, CLIP)), voices = decoder) }
        waitUntilAtLeastOneExists(hasContentDescription("Play clip.wav"), 5_000)
        onNodeWithContentDescription("Play clip.wav").performClick()
        val first = requireNotNull(decoder.latest)
        first.seek(4 * 60_000L)
        waitUntilAtLeastOneExists(hasText("4:00 / 10:00"), 5_000)

        first.dropped()
        waitUntilAtLeastOneExists(hasContentDescription("Try again, clip.wav"), 5_000)
        onNodeWithContentDescription("Try again, clip.wav").performClick()
        waitUntilAtLeastOneExists(hasContentDescription("Pause clip.wav"), 5_000)

        val second = requireNotNull(decoder.latest)
        assertEquals(node.minted, listOf(first.url, second.url))
        assertEquals(4 * 60_000L, second.positionMs)
        assertTrue(second.playing, "the new stream was not played on")
    }

    @Test
    fun aFilmWhoseStreamDroppedSaysSoAndShowsAgainOnANewTicket() = runComposeUiTest {
        val node = Ticketing(film())
        val projector = Projector()
        setContent { Stage(node, Stack(filePlace(PANE, FILM)), films = projector) }
        waitUntilAtLeastOneExists(hasContentDescription("Playing film.mp4"), 5_000)

        projector.dropped()
        waitUntilAtLeastOneExists(hasText("stopped playing", substring = true), 5_000)
        onNodeWithContentDescription("Try again, film.mp4").performClick()
        waitUntilAtLeastOneExists(hasContentDescription("Playing film.mp4"), 5_000)

        assertEquals(2, node.minted.size)
        assertEquals(node.minted.last(), projector.shown)
    }

    // The node turned the ticket down — it was restarting — and is asked again when the reader says.
    @Test
    fun aStreamTheNodeRefusedIsAskedForAgain() = runComposeUiTest {
        for ((answers, place, playing) in listOf(
            Triple(clip(), CLIP, "Play clip.wav"),
            Triple(film(), FILM, "Playing film.mp4"),
        )) {
            val node = Ticketing(answers, refusals = 1)
            setContent { Stage(node, Stack(filePlace(PANE, place)), films = Projector()) }
            waitUntilAtLeastOneExists(hasText("could not stream", substring = true), 5_000)
            onNodeWithText("Try again").performClick()
            waitUntilAtLeastOneExists(hasContentDescription(playing), 5_000)
            assertEquals(1, node.minted.size)
        }
    }
}
