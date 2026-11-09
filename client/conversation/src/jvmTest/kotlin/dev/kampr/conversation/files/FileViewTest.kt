package dev.kampr.conversation.files

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.doubleClick
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pinch
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.test.waitUntilAtLeastOneExists
import dev.kampr.conversation.AttachmentCard
import dev.kampr.conversation.AttachmentStore
import dev.kampr.conversation.Decoder
import dev.kampr.conversation.HeldClipboard
import dev.kampr.conversation.Projector
import dev.kampr.conversation.pngBytes
import dev.kampr.shared.model.Lookup
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.AttachmentSlice
import dev.kampr.shared.net.diffAttachmentId
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.net.findAttachmentId
import dev.kampr.shared.net.lsAttachmentId
import dev.kampr.shared.net.sliceOfWhole
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.filePlace
import dev.kampr.shared.ui.recordPlace
import dev.kampr.shared.wire.Attachment
import java.nio.file.Files
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w3:p2"
private const val NOTES = "/home/u/demo/notes.kt"
private const val SHOT = "/home/u/demo/shot.png"
private const val CLIP = "/home/u/demo/clip.wav"
private const val FILM = "/home/u/demo/film.mp4"
private const val SHEET = "/home/u/demo/people.csv"
private const val KIT = "/home/u/demo/kit.jar"
private const val BODY = "fun main() {\n    println(\"the fourth hop drops it\")\n}"
private const val PATCH = """diff --git a/notes.kt b/notes.kt
index 1111111..2222222 100644
--- a/notes.kt
+++ b/notes.kt
@@ -1,3 +1,3 @@
 fun main() {
-    println("old")
+    println("the fourth hop drops it")
 }
"""

private val WRITTEN = Lookup("kampr-node/fs.kt", "/home/u/demo", emptyList())

private const val VIEWER = "shot.png, pinch to zoom, double tap to fit"

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.open(answers: Map<String, AttachmentBytes>, place: FilesPlace): Pair<FileNode, Stack> {
    val node = FileNode(answers)
    val nav = Stack(place)
    setContent { Stage(node, nav) }
    return node to nav
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.zoomState(): String = onNodeWithContentDescription(VIEWER).fetchSemanticsNode().config[SemanticsProperties.StateDescription]

@OptIn(ExperimentalTestApi::class)
class FileViewTest {
    @Test
    fun aPathIsFetchedFromTheNodeItIsOnAndItsWordsShown() = runComposeUiTest {
        val (node, _) = open(mapOf(fileAttachmentId(NOTES) to text(BODY)), filePlace(PANE, NOTES))
        waitUntilAtLeastOneExists(hasText("the fourth hop drops it", substring = true), 5_000)
        assertEquals(PANE to fileAttachmentId(NOTES), node.asked.first())
    }

    @Test
    fun theWordsCanBeSelected() = runComposeUiTest {
        open(mapOf(fileAttachmentId(NOTES) to text(BODY)), filePlace(PANE, NOTES))
        waitUntilAtLeastOneExists(hasText("the fourth hop drops it", substring = true), 5_000)
        val body = onNodeWithText("the fourth hop drops it", substring = true, useUnmergedTree = true).fetchSemanticsNode()
        assertEquals(TEXT_CURSOR, body.cursor(), "the file body is not selectable")
    }

    @Test
    fun theBarClearsTheStatusBarItIsDrawnUnder() = runComposeUiTest {
        open(mapOf(fileAttachmentId(NOTES) to text(BODY)), filePlace(PANE, NOTES))
        waitUntilAtLeastOneExists(hasContentDescription("Close notes.kt"), 5_000)
        val close = onNodeWithContentDescription("Close notes.kt").getUnclippedBoundsInRoot()
        assertTrue(close.top >= PHONE_BARS.top, "close sits at ${close.top}, under the status bar")
    }

    @Test
    fun copyTakesTheWholeFileWithItsLineEndings() = runComposeUiTest {
        val whole = (0 until 2_100).joinToString("\r\n", postfix = "\r\n") { "line $it" }
        val board = HeldClipboard()
        val node = FileNode(mapOf(fileAttachmentId(NOTES) to text(whole)))
        val nav = Stack(filePlace(PANE, NOTES))
        setContent { WithClipboard(board) { Stage(node, nav) } }
        waitUntilAtLeastOneExists(hasContentDescription("Copy notes.kt"), 5_000)
        onNodeWithContentDescription("Copy notes.kt").performClick()
        waitUntil(timeoutMillis = 5_000) { board.held != null }
        assertEquals(whole, board.held)
    }

    @Test
    fun theChangesTabCountsWhatGitSaysAndShowsItSideBySide() = runComposeUiTest {
        open(
            mapOf(fileAttachmentId(NOTES) to text(BODY), diffAttachmentId(NOTES) to AttachmentBytes.Ok(PATCH.encodeToByteArray(), "text/plain")),
            filePlace(PANE, NOTES),
        )
        waitUntilAtLeastOneExists(hasContentDescription("Changes +1 −1 tab"), 5_000)
        onNodeWithContentDescription("Changes +1 −1 tab").performClick()
        waitForIdle()
        onNodeWithText("+    println(\"the fourth hop drops it\")", substring = true).assertExists()
        onNodeWithContentDescription("Side by side diff").performClick()
        waitForIdle()
        onNodeWithText("    println(\"old\")").assertExists()
        onNodeWithText("    println(\"the fourth hop drops it\")").assertExists()
    }

    @Test
    fun aFileGitHasNothingToSayAboutSaysSo() = runComposeUiTest {
        open(mapOf(fileAttachmentId(NOTES) to text(BODY)), filePlace(PANE, NOTES))
        waitUntilAtLeastOneExists(hasContentDescription("Changes tab"), 5_000)
        onNodeWithContentDescription("Changes tab").performClick()
        waitUntilAtLeastOneExists(hasText("No changes since HEAD", substring = true), 5_000)
    }

    @Test
    fun aRefusalIsSaidInTheNodesOwnWords() = runComposeUiTest {
        open(emptyMap(), filePlace(PANE, NOTES))
        waitUntilAtLeastOneExists(hasText("no such attachment"), 5_000)
    }

    // A tapped relative path that is not where it was written is looked for, and the viewer is
    // replaced by the file the node found rather than stopping at a refusal.
    @Test
    fun aRelativePathThatIsNotWhereItWasWrittenOpensWhereTheNodeFindsIt() = runComposeUiTest {
        val actual = "/home/u/demo/crates/kampr-node/fs.kt"
        val found = """{"matches":["$actual"],"truncated":false}""".encodeToByteArray()
        val (_, nav) = open(
            mapOf(findAttachmentId("/home/u/demo", "kampr-node/fs.kt") to AttachmentBytes.Ok(found, "application/json"), fileAttachmentId(actual) to text(BODY)),
            filePlace(PANE, "/home/u/demo/kampr-node/fs.kt", WRITTEN),
        )
        waitUntilAtLeastOneExists(hasText("the fourth hop drops it", substring = true), 5_000)
        assertEquals(listOf<FilesPlace>(filePlace(PANE, actual)), nav.places.toList())
    }

    @Test
    fun aRelativePathFoundNowhereIsTheSameRefusal() = runComposeUiTest {
        open(emptyMap(), filePlace(PANE, "/home/u/demo/kampr-node/fs.kt", WRITTEN))
        waitUntilAtLeastOneExists(hasText("no such attachment"), 5_000)
    }

    @Test
    fun aPathThatIsADirectoryReplacesItselfWithTheListing() = runComposeUiTest {
        val (_, nav) = open(
            mapOf(lsAttachmentId("/etc") to AttachmentBytes.Ok("{}".encodeToByteArray(), "application/json")),
            filePlace(PANE, "/etc"),
        )
        waitUntil(timeoutMillis = 5_000) { nav.places.firstOrNull() is FilesPlace.Dir }
        assertEquals(listOf<FilesPlace>(FilesPlace.Dir("01JNODE", PANE, "/etc")), nav.places.toList())
    }

    @Test
    fun theFolderActionOpensTheParentAboveTheFile() = runComposeUiTest {
        val (_, nav) = open(mapOf(fileAttachmentId(NOTES) to text(BODY)), filePlace(PANE, NOTES))
        onNodeWithContentDescription("Show the folder holding notes.kt").performClick()
        assertEquals(FilesPlace.Dir("01JNODE", PANE, "/home/u/demo"), nav.places.last())
        assertEquals(2, nav.places.size, "the file was dropped rather than kept to come back to")
    }

    @Test
    fun bytesThatAreNeitherWordsNorAPictureAreOfferedAsADownload() = runComposeUiTest {
        val blob = byteArrayOf(0, 1, 2, 3, -1, -2)
        open(mapOf(fileAttachmentId("/home/u/a.bin") to AttachmentBytes.Ok(blob, "application/octet-stream")), filePlace(PANE, "/home/u/a.bin"))
        waitUntilAtLeastOneExists(hasContentDescription("Download a.bin to this device"), 5_000)
        onNodeWithText("cannot show", substring = true).assertExists()
    }

    @Test
    fun aPictureDownloadsAndSaysWhereItLanded() = runComposeUiTest {
        val home = Files.createTempDirectory("kampr-home").toFile()
        val was = System.getProperty("user.home")
        System.setProperty("user.home", home.path)
        try {
            open(mapOf(fileAttachmentId(SHOT) to AttachmentBytes.Ok(pngBytes(64, 64), "image/png")), filePlace(PANE, SHOT))
            waitUntilAtLeastOneExists(hasContentDescription("Download shot.png to this device"), 5_000)
            onNodeWithContentDescription("Download shot.png to this device").performClick()
            waitUntilAtLeastOneExists(hasContentDescription("Saved to ${home.path}/shot.png"), 5_000)
            assertTrue(java.io.File(home, "shot.png").length() > 0)
        } finally {
            System.setProperty("user.home", was)
            home.deleteRecursively()
        }
    }

    @Test
    fun twoFingersSpreadingApartZoomThePictureIn() = runComposeUiTest {
        open(mapOf(fileAttachmentId(SHOT) to AttachmentBytes.Ok(pngBytes(900, 1400), "image/png")), filePlace(PANE, SHOT))
        waitUntilAtLeastOneExists(hasContentDescription(VIEWER), 5_000)
        assertEquals("fitted", zoomState())
        onNodeWithContentDescription(VIEWER).performTouchInput {
            pinch(center - Offset(20f, 0f), center - Offset(200f, 0f), center + Offset(20f, 0f), center + Offset(200f, 0f))
        }
        waitForIdle()
        assertTrue(zoomState().startsWith("zoomed"), "a pinch left the picture ${zoomState()}")
    }

    @Test
    fun aDoubleTapZoomsInAndTheNextFits() = runComposeUiTest {
        open(mapOf(fileAttachmentId(SHOT) to AttachmentBytes.Ok(pngBytes(900, 1400), "image/png")), filePlace(PANE, SHOT))
        waitUntilAtLeastOneExists(hasContentDescription(VIEWER), 5_000)
        onNodeWithContentDescription(VIEWER).performTouchInput { doubleClick(center) }
        waitForIdle()
        assertEquals("zoomed 250%", zoomState())
        onNodeWithContentDescription(VIEWER).performTouchInput { doubleClick(center) }
        waitForIdle()
        assertEquals("fitted", zoomState())
    }

    // A recording is played from the node, not read into memory first: the viewer looks at the
    // first slice to learn what the file is, and then hands the player a stream of it.
    @Test
    fun aRecordingOpensAsAPlayerThatStreamsFromTheNode() = runComposeUiTest {
        val decoder = Decoder()
        val node = FileNode(mapOf(fileAttachmentId(CLIP) to AttachmentBytes.Ok(ByteArray(64) { it.toByte() }, "audio/wav")))
        setContent { Stage(node, Stack(filePlace(PANE, CLIP)), voices = decoder) }
        waitUntilAtLeastOneExists(hasContentDescription("Play clip.wav"), 5_000)
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitUntilAtLeastOneExists(hasContentDescription("Pause clip.wav"), 5_000)

        assertEquals(streamOf(fileAttachmentId(CLIP)), decoder.latest?.url)
        assertTrue(decoder.latest?.playing == true, "the press reached no player")
        onNodeWithContentDescription("Download clip.wav to this device").assertExists()
    }

    // The case streaming exists for: an hour of audio is hundreds of megabytes, and none of it past
    // the first slice is ever asked for by the viewer.
    @Test
    fun aLongRecordingIsNeverReadToItsEnd() = runComposeUiTest {
        val ranges = mutableListOf<Long>()
        val node = object : PaneIo by FileNode(emptyMap()) {
            override suspend fun range(paneId: String, id: String, start: Long, endInclusive: Long): AttachmentSlice {
                ranges += start
                return AttachmentSlice.Ok(ByteArray(64) { it.toByte() }, "audio/wav", 600_000_000L)
            }
        }
        setContent { Stage(node, Stack(filePlace(PANE, CLIP))) }
        waitUntilAtLeastOneExists(hasContentDescription("Play clip.wav"), 5_000)
        assertEquals(listOf(0L), ranges, "the viewer read past the first slice of a recording it streams")
    }

    @Test
    fun aRecordingThisDeviceCannotPlayIsOfferedAsADownload() = runComposeUiTest {
        val node = FileNode(mapOf(fileAttachmentId(CLIP) to AttachmentBytes.Ok(ByteArray(64) { it.toByte() }, "audio/flac")))
        setContent { Stage(node, Stack(filePlace(PANE, CLIP))) }
        waitUntilAtLeastOneExists(hasContentDescription("Download clip.wav to this device"), 5_000)
        waitUntilAtLeastOneExists(hasText("cannot show", substring = true), 5_000)
    }

    // What Windows tools write by default: UTF-16 with a byte-order mark, every other byte a NUL. It
    // used to be offered as bytes this device cannot show.
    @Test
    fun textInAnEncodingOtherThanUtf8IsReadAndSaysWhichItWasReadAs() = runComposeUiTest {
        val table = listOf(
            Triple("/home/u/demo/log.txt", byteArrayOf(0xFF.toByte(), 0xFE.toByte()) + "build succeeded\r\n0 warnings\r\n".toByteArray(Charsets.UTF_16LE), "UTF-16"),
            Triple("/home/u/demo/old.txt", "“café” – naïve\n".toByteArray(charset("windows-1252")), "Windows-1252"),
        )
        val node = FileNode(table.associate { (path, bytes, _) -> fileAttachmentId(path) to AttachmentBytes.Ok(bytes, "application/octet-stream") })
        val nav = Stack(filePlace(PANE, table[0].first))
        setContent { Stage(node, nav) }
        waitUntilAtLeastOneExists(hasText("build succeeded", substring = true), 5_000)
        waitUntilAtLeastOneExists(hasText("UTF-16", substring = true), 5_000)

        nav.replace(filePlace(PANE, table[1].first))
        waitUntilAtLeastOneExists(hasText("“café” – naïve", substring = true), 5_000)
        waitUntilAtLeastOneExists(hasText("Windows-1252", substring = true), 5_000)
    }

    // A film is streamed like a recording: the first slice says what it is, and the screen is
    // pointed at a stream of the rest rather than handed hundreds of megabytes.
    @Test
    fun aFilmOpensOnAScreenThatStreamsFromTheNodeAndIsNeverReadToItsEnd() = runComposeUiTest {
        val ranges = mutableListOf<Long>()
        val node = object : PaneIo by FileNode(emptyMap()) {
            override suspend fun range(paneId: String, id: String, start: Long, endInclusive: Long): AttachmentSlice {
                ranges += start
                return AttachmentSlice.Ok(ByteArray(64) { it.toByte() }, "video/mp4", 900_000_000L)
            }
            override suspend fun streamUrl(paneId: String, id: String): String = streamOf(id)
        }
        val projector = Projector()
        setContent { Stage(node, Stack(filePlace(PANE, FILM)), films = projector) }
        waitUntilAtLeastOneExists(hasContentDescription("Playing film.mp4"), 5_000)

        assertEquals(streamOf(fileAttachmentId(FILM)), projector.shown)
        assertEquals(listOf(0L), ranges, "the viewer read past the first slice of a film it streams")
        onNodeWithContentDescription("Download film.mp4 to this device").assertExists()
    }

    @Test
    fun aFilmThisDeviceCannotPlayIsOfferedAsADownload() = runComposeUiTest {
        val node = FileNode(mapOf(fileAttachmentId(FILM) to AttachmentBytes.Ok(ByteArray(64) { it.toByte() }, "video/quicktime")))
        setContent { Stage(node, Stack(filePlace(PANE, FILM))) }
        waitUntilAtLeastOneExists(hasText("cannot show", substring = true), 5_000)
    }

    // The transcript's way to the same screen: an attachment whose type is a film opens the viewer.
    @Test
    fun aFilmAttachedToATurnOpensOnTheScreenToo() = runComposeUiTest {
        val att = Attachment(id = "att-film", kind = "file", mime = "video/mp4", name = "demo.mp4")
        val node = FileNode(mapOf("att-film" to AttachmentBytes.Ok(ByteArray(64) { it.toByte() }, "video/mp4")))
        val projector = Projector()
        setContent { Stage(node, Stack(recordPlace(PANE, att)), films = projector) }
        waitUntilAtLeastOneExists(hasContentDescription("Playing demo.mp4"), 5_000)
        assertEquals(streamOf("att-film"), projector.shown)
    }

    @Test
    fun pressingAFilmsCardInATranscriptOpensItOnTheScreen() = runComposeUiTest {
        val att = Attachment(id = "att-film", kind = "file", mime = "video/mp4", name = "demo.mp4")
        val node = FileNode(mapOf("att-film" to AttachmentBytes.Ok(ByteArray(64) { it.toByte() }, "video/mp4")))
        val projector = Projector()
        setContent {
            Stage(node, Stack(), films = projector) { AttachmentCard(att, AttachmentStore(PANE, Decoder(), projector)) }
        }
        onNodeWithContentDescription("Show video, demo.mp4").performClick()
        waitUntilAtLeastOneExists(hasContentDescription("Playing demo.mp4"), 5_000)
        assertEquals(streamOf("att-film"), projector.shown)
    }

    // Each cell is its own text, so a quoted comma and a doubled quote arrive as the values a
    // spreadsheet would show; and the source is a press away, as markdown's is.
    @Test
    fun aCsvIsShownAsATableOfItsCellsAndItsSourceIsAPressAway() = runComposeUiTest {
        val csv = "name;city;note\n\"Smith; J\";Perth;\"said \"\"hi\"\"\"\nLee;Hobart;*not italic*\n"
        val node = FileNode(mapOf(fileAttachmentId(SHEET) to text(csv)))
        setContent { Stage(node, Stack(filePlace(PANE, SHEET))) }
        waitUntilAtLeastOneExists(hasText("Smith; J"), 5_000)
        onNodeWithText("city").assertExists()
        onNodeWithText("said \"hi\"").assertExists()
        onNodeWithText("*not italic*").assertExists()

        onNodeWithContentDescription("Source", substring = true).performClick()
        waitUntilAtLeastOneExists(hasText("\"Smith; J\";Perth;\"said \"\"hi\"\"\"", substring = true), 5_000)
    }

    // A jar, an apk or a docx is a zip by its bytes whatever its name, and nothing in the node's
    // answer says so: a file on disk is served with no type.
    @Test
    fun anArchiveIsListedByWhatItHoldsAndStillDownloads() = runComposeUiTest {
        val kit = zipOf {
            folder("src/")
            file("src/main.kt", BODY.encodeToByteArray())
            file("README.md", "# kit".encodeToByteArray())
        }
        open(mapOf(fileAttachmentId(KIT) to AttachmentBytes.Ok(kit, "application/octet-stream")), filePlace(PANE, KIT))
        waitUntilAtLeastOneExists(hasText("src/main.kt"), 5_000)
        onNodeWithText("README.md").assertExists()
        onNodeWithText("src/").assertExists()
        onNodeWithText("2 files", substring = true).assertExists()
        onNodeWithContentDescription("Download kit.jar to this device").assertExists()
    }

    @Test
    fun anArchiveAttachedToATurnIsListedFromTheBytesInHand() = runComposeUiTest {
        val att = Attachment(id = "att-zip", kind = "file", name = "kit.zip")
        val kit = zipOf { file("inside.txt", byteArrayOf(1)) }
        open(mapOf("att-zip" to AttachmentBytes.Ok(kit, "application/zip")), recordPlace(PANE, att))
        waitUntilAtLeastOneExists(hasText("inside.txt"), 5_000)
    }

    // The directory sits at the end of the archive, past the first slice: it is read from there,
    // and the megabytes of the entries before it never are.
    @Test
    fun aBigArchiveIsListedFromItsTailAlone() = runComposeUiTest {
        val kit = zipOf {
            file("blob.bin", ByteArray(3 * SLICE_BYTES.toInt()) { (it * 31).toByte() }, stored = true)
            file("last.txt", byteArrayOf(1))
        }
        val reads = mutableListOf<LongRange>()
        val node = object : PaneIo by FileNode(emptyMap()) {
            override suspend fun range(paneId: String, id: String, start: Long, endInclusive: Long): AttachmentSlice {
                reads += start..endInclusive
                return sliceOfWhole(kit, null, start, endInclusive)
            }
        }
        setContent { Stage(node, Stack(filePlace(PANE, KIT))) }
        waitUntilAtLeastOneExists(hasText("last.txt"), 5_000)
        onNodeWithText("blob.bin").assertExists()
        assertTrue(reads.drop(1).all { it.first > SLICE_BYTES }, "read in the middle of the archive: $reads")
        assertTrue(reads.drop(1).sumOf { it.last - it.first + 1 } < 70_000, "read too much of the tail: $reads")
    }

    @Test
    fun anArchiveTooLongToListWholeSaysHowMuchIsShown() = runComposeUiTest {
        val kit = zipOf { repeat(ZIP_ENTRY_CAP + 1) { file("f$it", ByteArray(0), stored = true) } }
        open(mapOf(fileAttachmentId(KIT) to AttachmentBytes.Ok(kit, "application/octet-stream")), filePlace(PANE, KIT))
        waitUntilAtLeastOneExists(hasText("f0"), 10_000)
        onNodeWithText("Showing the first 50,000 of 50,001 entries", substring = true).assertExists()
    }

    @Test
    fun aDamagedArchiveIsOfferedAsADownloadSayingWhy() = runComposeUiTest {
        val kit = zipOf { file("a.txt", byteArrayOf(1)) }.let { it.copyOf(it.size - 10) }
        open(mapOf(fileAttachmentId(KIT) to AttachmentBytes.Ok(kit, "application/octet-stream")), filePlace(PANE, KIT))
        waitUntilAtLeastOneExists(hasText("could not be listed", substring = true), 5_000)
        onNodeWithContentDescription("Download kit.jar to this device").assertExists()
    }
}
