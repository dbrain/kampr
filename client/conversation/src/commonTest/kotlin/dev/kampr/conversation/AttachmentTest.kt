package dev.kampr.conversation

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val PANE = "01JNODE.../w3:p2"

// Hand-written against the fixed contract rather than captured, because the node that produces an
// `att` does not exist yet. The third turn is the one that matters most: an unknown kind, two
// fields this release has never heard of, and one known field of the wrong shape.
private const val ATTACHED_CONVO =
    "{\"cursor\":\"i-1\",\"more\":false,\"pane\":\"$PANE\",\"t\":\"convo\",\"turns\":[" +
        "{\"at\":\"2026-08-24T09:00:01.000Z\",\"blocks\":[" +
        "{\"b\":\"md\",\"text\":\"look at this\"}," +
        "{\"b\":\"md\",\"text\":\"[image · png]\",\"att\":{\"id\":\"att-7f3\",\"kind\":\"image\"," +
        "\"mime\":\"image/png\",\"bytes\":52831,\"name\":\"shot.png\"}}]," +
        "\"id\":\"i-1\",\"role\":\"user\"}," +
        "{\"at\":\"2026-08-24T09:00:02.000Z\",\"blocks\":[" +
        "{\"b\":\"md\",\"text\":\"[image · png]\",\"att\":{\"id\":\"att-pasted\",\"kind\":\"image\"," +
        "\"mime\":\"image/png\",\"bytes\":731004}}]," +
        "\"id\":\"i-2\",\"role\":\"user\"}," +
        "{\"at\":\"2026-08-24T09:00:03.000Z\",\"blocks\":[" +
        "{\"b\":\"md\",\"text\":\"[file · zip]\",\"att\":{\"id\":\"att-zip\",\"kind\":\"archive\"," +
        "\"mime\":\"application/zip\",\"bytes\":{\"n\":1400000},\"name\":\"logs.zip\"," +
        "\"sha256\":\"9f1c\",\"preview\":{\"w\":10,\"h\":10}}}]," +
        "\"id\":\"i-3\",\"role\":\"user\"}]}"

private const val HEADERLESS_CONVO =
    "{\"cursor\":\"p-1\",\"more\":false,\"pane\":\"$PANE\",\"t\":\"convo\",\"turns\":[" +
        "{\"at\":\"2026-08-24T09:00:01.000Z\",\"blocks\":[{\"b\":\"md\",\"text\":\"plain prose\"}]," +
        "\"id\":\"p-1\",\"role\":\"assistant\"}]}"

// An `att` with no id is not a handle for anything, so it is not an attachment: the block keeps
// its marker and reads as the prose it always was.
private const val IDLESS_CONVO =
    "{\"cursor\":\"n-1\",\"more\":false,\"pane\":\"$PANE\",\"t\":\"convo\",\"turns\":[" +
        "{\"at\":\"2026-08-24T09:00:01.000Z\",\"blocks\":[{\"b\":\"md\",\"text\":\"[image · png]\"," +
        "\"att\":{\"kind\":\"image\",\"mime\":\"image/png\"}}]," +
        "\"id\":\"n-1\",\"role\":\"user\"}]}"

private fun turnsOf(frame: String) =
    KamprStore().also { it.accept(assertNotNull(Wire.decode(frame), "undecodable frame")) }.pane(PANE).turns

class AttachmentTest {
    @Test
    fun anImageHeaderRidesBesideItsMarkerAndBecomesAPieceOfItsOwn() {
        val blocks = turnsOf(ATTACHED_CONVO).first { it.id == "i-1" }.blocks
        val marked = blocks.filterIsInstance<Block.Md>().last()
        assertEquals("[image · png]", marked.text, "the marker is unchanged for a client that ignores att")
        val att = assertNotNull(marked.att)
        assertEquals("att-7f3", att.id)
        assertEquals("image", att.kind)
        assertEquals("image/png", att.mime)
        assertEquals(52831L, att.bytes)
        assertEquals("shot.png", att.name)

        val pieces = groupBlocks(blocks)
        assertEquals(Piece.Prose("look at this"), pieces.first())
        assertEquals(att, (pieces.last() as Piece.Attach).att)
    }

    @Test
    fun aProseBlockWithNoHeaderIsUnchanged() {
        val blocks = turnsOf(HEADERLESS_CONVO).single().blocks
        assertNull(blocks.filterIsInstance<Block.Md>().single().att)
        assertEquals(listOf(Piece.Prose("plain prose")), groupBlocks(blocks))
    }

    // The rule the whole design rests on: a kind nobody has written a viewer for is a file, not a
    // block that disappears out of the transcript on every phone already installed.
    @Test
    fun aKindThisClientHasNeverSeenIsOfferedAsADownloadRatherThanDropped() {
        val blocks = turnsOf(ATTACHED_CONVO).first { it.id == "i-3" }.blocks
        val att = assertNotNull(blocks.filterIsInstance<Block.Md>().single().att)
        assertEquals("archive", att.kind)
        assertEquals(AttachmentOffer.File, offerFor(att))
        assertEquals("Download file", offerFor(att).label)
        assertTrue(groupBlocks(blocks).single() is Piece.Attach)
    }

    @Test
    fun anAttachmentWithFieldsAndShapesThisReleaseDoesNotKnowStillYieldsItsId() {
        val att = assertNotNull(
            turnsOf(ATTACHED_CONVO).first { it.id == "i-3" }.blocks
                .filterIsInstance<Block.Md>().single().att,
            "an att carrying sha256, preview and a bytes of the wrong shape took the whole turn down",
        )
        assertEquals("att-zip", att.id)
        assertEquals("logs.zip", att.name)
        assertNull(att.bytes, "a known field of an unknown shape is absent, not fatal")
    }

    @Test
    fun anAttachmentWithNoHandleIsNotAnAttachment() {
        val blocks = turnsOf(IDLESS_CONVO).single().blocks
        assertNull(blocks.filterIsInstance<Block.Md>().single().att)
        assertEquals(listOf(Piece.Prose("[image · png]")), groupBlocks(blocks))
    }

    // A pasted screenshot has no filename at all, which is the ordinary case rather than an edge.
    @Test
    fun aHeaderWithNoNameStillSaysWhatItIsAndHowBigItIs() {
        val att = assertNotNull(
            turnsOf(ATTACHED_CONVO).first { it.id == "i-2" }.blocks
                .filterIsInstance<Block.Md>().single().att,
        )
        assertEquals("Image", headlineOf(att))
        assertEquals("png · 731 KB", detailOf(att))
        assertEquals(AttachmentOffer.Image, offerFor(att))
    }

    // The counter in the transcript bar promises the reader somewhere to go. A hit on a marker
    // that is no longer rendered is a promise it cannot keep.
    @Test
    fun searchFindsAnAttachmentByItsNameRatherThanByTheMarkerItReplaced() {
        val turns = turnsOf(ATTACHED_CONVO)
        assertEquals(listOf(0), searchHits(turns, "shot.png"))
        assertEquals(listOf(2), searchHits(turns, "logs.zip"))
        assertEquals(listOf(0, 1), searchHits(turns, "image/png"), "the mime is worth finding, the marker is not")
        assertTrue(searchHits(turns, "[image").isEmpty(), "a match nothing on screen carries")
    }

    @Test
    fun aNamedHeaderLeadsWithItsName() {
        val att = assertNotNull(
            turnsOf(ATTACHED_CONVO).first { it.id == "i-1" }.blocks
                .filterIsInstance<Block.Md>().last().att,
        )
        assertEquals("shot.png", headlineOf(att))
        assertEquals("png · 52.8 KB", detailOf(att))
    }
}
