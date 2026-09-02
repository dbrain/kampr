package dev.kampr.conversation

import dev.kampr.shared.wire.Attachment
import kotlin.test.Test
import kotlin.test.assertEquals

// An offer existed for `kind == "video"` and nothing in the tree has ever written that kind. Had a
// node begun to, the reader would have been shown "Show video" over a route that serves no video
// type inline and a client with no player — the present-and-failing affordance the house rule
// forbids, waiting for its producer to arrive.
//
// So video takes the download every unknown kind takes. The type is still *named* in the card's
// detail line, because saying what a thing is costs nothing and promises nothing.
class VideoOfferTest {
    @Test
    fun aVideoIsOfferedAsADownloadRatherThanAsSomethingThisClientCanPlay() {
        val clip = Attachment(id = "a1", kind = "video", mime = "video/mp4", name = "demo.mp4")
        assertEquals(AttachmentOffer.File, offerFor(clip))
        assertEquals("Download file", offerFor(clip).label)
    }

    @Test
    fun aTypeIsStillNamedEvenWhereItIsNotOffered() {
        val clip = Attachment(id = "a1", kind = "video", mime = "video/mp4", name = "demo.mp4", bytes = 2048)
        assertEquals("mp4 · 2 KB", detailOf(clip))
    }
}
