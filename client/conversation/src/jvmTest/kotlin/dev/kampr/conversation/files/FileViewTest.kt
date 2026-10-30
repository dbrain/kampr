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
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.waitUntilAtLeastOneExists
import androidx.compose.ui.text.AnnotatedString
import dev.kampr.conversation.pngBytes
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.diffAttachmentId
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.net.lsAttachmentId
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.filePlace
import java.nio.file.Files
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w3:p2"
private const val NOTES = "/home/u/demo/notes.kt"
private const val SHOT = "/home/u/demo/shot.png"
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
        val board = Pasteboard()
        val node = FileNode(mapOf(fileAttachmentId(NOTES) to text(whole)))
        val nav = Stack(filePlace(PANE, NOTES))
        setContent { WithClipboard(board) { Stage(node, nav) } }
        waitUntilAtLeastOneExists(hasContentDescription("Copy notes.kt"), 5_000)
        onNodeWithContentDescription("Copy notes.kt").performClick()
        waitUntil(timeoutMillis = 5_000) { board.held != null }
        assertEquals(AnnotatedString(whole), board.held)
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
}
