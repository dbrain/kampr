package dev.kampr.conversation

import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.wire.Attachment
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals

// What the node writes for a `SendUserFile` call: the caption as prose and each file as a header
// whose id is the path, kind decided from the extension alone.
private fun sentTurn(id: String, vararg paths: String): Turn = Turn(
    id = id,
    role = "assistant",
    at = null,
    blocks = listOf(Block.Md("Before and after.")) + paths.map { path ->
        val picture = path.endsWith(".jpg")
        Block.Md(
            path,
            Attachment(
                id = fileAttachmentId(path),
                kind = if (picture) "image" else "file",
                mime = if (picture) "image/jpeg" else null,
                name = fileName(path),
            ),
        )
    },
)

class SentFileTest {
    @Test
    fun aSentFileBetweenToolCallsIsARowOfItsOwnAndNotPartOfTheRun() {
        val turns = listOf(
            bashTurn("s-1", "make shots", "done", 1),
            bashTurn("s-2", "ls shots", "done", 4),
            sentTurn("s-3", "/tmp/shot.jpg"),
            bashTurn("s-4", "rm -r shots", "done", 0),
        )
        val rows = transcriptRows(turns, "").filter { it !is TranscriptRow.Head }
        assertEquals(listOf("s-1", "s-2", "s-3", "s-4"), rows.map { it.key })
    }

    @Test
    fun sentPicturesSideBySideAreOneGalleryAndASentFileIsOfferedTheWayAPathOnACardIs() {
        val pieces = groupBlocks(
            sentTurn("s-1", "/tmp/before.jpg", "/tmp/after.jpg", "/home/u/report.md", "/tmp/a.zip").blocks,
        )
        assertEquals(
            listOf(
                Piece.Prose("Before and after."),
                Piece.Gallery(listOf(fileTarget("/tmp/before.jpg"), fileTarget("/tmp/after.jpg"))),
                Piece.Attach(fileTarget("/home/u/report.md")),
                Piece.Attach(fileTarget("/tmp/a.zip")),
            ),
            pieces,
        )
        assertEquals(AttachmentOffer.Text, offerFor((pieces[2] as Piece.Attach).att))
    }
}
