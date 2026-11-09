package dev.kampr.conversation

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import dev.kampr.shared.wire.Attachment
import kotlin.test.Test
import kotlin.test.assertEquals

private object NoVoices : Voices {
    override fun canPlay(mime: String?) = false
    override fun stream(url: String, mime: String?): Voice = error("nothing here plays")
}

private class FilmsOf(private val types: Set<String>) : Films {
    override fun canPlay(mime: String?) = videoType(mime) in types

    @Composable
    override fun Screen(url: String, name: String, onFailed: () -> Unit, modifier: Modifier) = Unit
}

// A film is something to look at, so it opens the viewer — and only where this device has a player
// for its type, or "Show video" is a present-and-failing affordance over a download.
class VideoOfferTest {
    @Test
    fun aFilmIsOfferedToWatchOnlyWhereTheDeviceCanPlayIt() {
        for (clip in listOf(
            Attachment(id = "a1", kind = "video", mime = "video/mp4", name = "demo.mp4"),
            Attachment(id = "a2", kind = "file", mime = "video/mp4", name = "demo.mp4"),
        )) {
            assertEquals(AttachmentOffer.Video, offerFor(clip))
            assertEquals(AttachmentOffer.Video, offerOn(clip, NoVoices, FilmsOf(setOf("video/mp4"))))
            assertEquals(AttachmentOffer.File, offerOn(clip, NoVoices, FilmsOf(emptySet())))
            assertEquals("Download file", offerOn(clip, NoVoices, FilmsOf(emptySet())).label)
        }
    }

    @Test
    fun aTypeIsStillNamedEvenWhereItIsNotOffered() {
        val clip = Attachment(id = "a1", kind = "video", mime = "video/mp4", name = "demo.mp4", bytes = 2048)
        assertEquals("mp4 · 2 KB", detailOf(clip))
    }
}
