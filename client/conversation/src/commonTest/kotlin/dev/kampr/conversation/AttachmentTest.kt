package dev.kampr.conversation

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.Attachment
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
        assertEquals(listOf(0), hitRows(turns, "shot.png"))
        assertEquals(listOf(2), hitRows(turns, "logs.zip"))
        assertEquals(listOf(0, 1), hitRows(turns, "image/png"), "the mime is worth finding, the marker is not")
        assertTrue(hitRows(turns, "[image").isEmpty(), "a match nothing on screen carries")
    }

    // The report: an agent wrote a `.wav`, and the conversation pane showed nothing to press but
    // an offer to read it as a document. Everything that was not a picture was typed `text`.
    @Test
    fun aRecordingIsTypedAsOneRatherThanAsADocumentToRead() {
        val wav = fileTarget("/home/u/demo/clip.wav")
        assertEquals("audio", wav.kind)
        assertEquals("clip.wav", wav.name)
        assertEquals(AttachmentOffer.Audio, offerFor(wav))
        assertNull(wav.mime, "an extension's guess must not outrank the type the node read off the bytes")
    }

    @Test
    fun aPathIsTypedByItsExtensionAndEverythingUnnamedStaysAFileToRead() {
        for ((path, kind) in listOf(
            "/tmp/shot.png" to "image",
            "/tmp/a.WAV" to "audio",
            "/tmp/a.mp3" to "audio",
            "/tmp/a.flac" to "audio",
            "/tmp/a.opus" to "audio",
            "/tmp/logs.zip" to "file",
            "/tmp/clip.mp4" to "file",
            "/tmp/main.rs" to "text",
            "/tmp/README" to "text",
            "/tmp/notes.no-such-type" to "text",
        )) assertEquals(kind, fileTarget(path).kind, path)
    }

    // Additive by rule: a node that starts recording `audio/wav` beside a record reaches the same
    // player on a client already installed, without a new `kind` and without a new `t`.
    @Test
    fun aRecordedAudioTypeIsEnoughOnItsOwn() {
        val att = Attachment(id = "att-1", kind = "file", mime = "audio/wav", name = "clip.wav")
        assertEquals(AttachmentOffer.Audio, offerFor(att))
        assertEquals("wav · 12 B", detailOf(att.copy(bytes = 12)))
    }

    // The narrow rule that makes reading prose for paths defensible at all, applied to the kind
    // this adds: a token has to be a path *and* be named as a recording.
    @Test
    fun onlyAPathThatNamesARecordingIsOfferedAsOne() {
        assertEquals(listOf("/home/u/demo/clip.wav"), soundsIn("rendered `/home/u/demo/clip.wav` for you"))
        assertEquals(emptyList(), soundsIn("I made you a wav file of it"))
        assertEquals(emptyList(), soundsIn("see out/clip.wav"), "a relative path is not one the route resolves")
        assertEquals(emptyList(), soundsIn("see /home/u/demo/notes.md"))
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
