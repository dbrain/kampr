package dev.kampr.conversation.files

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.waitUntilAtLeastOneExists
import androidx.compose.ui.text.AnnotatedString
import dev.kampr.conversation.pngBytes
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.AttachmentSlice
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.filePlace
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w3:p2"
private const val BIG = "/home/u/demo/big.log"
private const val LINES = 30_000
private const val SHOT = "/home/u/demo/shot.png"

// Two- and three-byte characters on every line, so slices that end where the node's ceiling
// puts them land inside a character over and over.
private fun line(n: Int) = "line $n — ünïcödé ✓ ${"x".repeat(n % 7)}"

private val WHOLE = (1..LINES).joinToString("\n", postfix = "\n") { line(it) }.encodeToByteArray()

// A node that serves at most `most` bytes per range, as a node behind a slow relay may, and
// counts every time anything asked for the whole file instead.
private class RangedNode(
    private val bytes: ByteArray,
    private val most: Int = 64 * 1024,
    private val path: String = BIG,
    private val mime: String = "text/plain",
) : PaneIo {
    var wholes = 0
    var downloaded: String? = null
    override suspend fun download(paneId: String, id: String, name: String): String? {
        downloaded = name
        return "Downloads/$name"
    }
    val ranges = mutableListOf<LongRange>()
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes {
        if (id != fileAttachmentId(path)) return AttachmentBytes.Failed("no such attachment")
        wholes++
        return AttachmentBytes.Ok(bytes, mime)
    }
    override suspend fun range(paneId: String, id: String, start: Long, endInclusive: Long): AttachmentSlice {
        if (id != fileAttachmentId(path)) return AttachmentSlice.Failed("no such attachment")
        val end = minOf(endInclusive, start + most - 1, bytes.size - 1L)
        ranges += start..end
        return AttachmentSlice.Ok(bytes.copyOfRange(start.toInt(), end.toInt() + 1), mime, bytes.size.toLong())
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.scrollToTheEnd(until: String) {
    val list = SemanticsMatcher.keyIsDefined(SemanticsProperties.IndexForKey)
    repeat(200) {
        if (onAllNodesWithText(until).fetchSemanticsNodes().isNotEmpty()) return
        val node = onNode(list).fetchSemanticsNode()
        val rows = node.config[SemanticsProperties.CollectionInfo].rowCount
        onNode(list).performScrollToIndex(rows - 1)
        waitForIdle()
    }
}

@OptIn(ExperimentalTestApi::class)
class FileStreamTest {
    @Test
    fun aThirtyThousandLineFileReadsToItsLastLineInSlicesWithTheRightNumberBesideIt() = runComposeUiTest {
        val node = RangedNode(WHOLE)
        val nav = Stack(filePlace(PANE, BIG))
        setContent { Stage(node, nav) }
        waitUntilAtLeastOneExists(hasText(line(1)), 5_000)
        scrollToTheEnd(line(LINES))
        val last = onNodeWithText(line(LINES)).getUnclippedBoundsInRoot()
        val number = onNodeWithText("$LINES").getUnclippedBoundsInRoot()
        assertTrue(abs(last.top.value - number.top.value) < 2f, "line $LINES sits at ${last.top}, its number at ${number.top}")
        onAllNodesWithText("${LINES + 1}").assertCountEquals(0)
        onAllNodesWithText("�", substring = true).assertCountEquals(0)
        assertEquals(0, node.wholes, "the viewer asked for the whole file")
        assertTrue(node.ranges.size > 10, "only ${node.ranges.size} slices were asked for")
    }

    @Test
    fun copyFetchesWhatIsNotLoadedYetAndTakesTheWholeFile() = runComposeUiTest {
        val board = Pasteboard()
        val node = RangedNode(WHOLE)
        val nav = Stack(filePlace(PANE, BIG))
        setContent { WithClipboard(board) { Stage(node, nav) } }
        waitUntilAtLeastOneExists(hasContentDescription("Copy all of big.log"), 5_000)
        onNodeWithContentDescription("Copy all of big.log").performClick()
        waitUntil(timeoutMillis = 10_000) { board.held != null }
        assertEquals(AnnotatedString(WHOLE.decodeToString()), board.held)
    }

    @Test
    fun aFileThatFitsInOneSliceIsShownWhole() = runComposeUiTest {
        val small = "alpha\nbeta\ngamma".encodeToByteArray()
        setContent { Stage(RangedNode(small), Stack(filePlace(PANE, BIG))) }
        waitUntilAtLeastOneExists(hasText("gamma"), 5_000)
        onNodeWithText("3").assertExists()
        onAllNodesWithText("4").assertCountEquals(0)
        onNodeWithContentDescription("Copy big.log").assertExists()
    }

    @Test
    fun aPictureLargerThanOneSliceIsFetchedToItsEndAndDrawn() = runComposeUiTest {
        val png = pngBytes(900, 1400)
        val node = RangedNode(png, most = 1024, path = SHOT, mime = "image/png")
        setContent { Stage(node, Stack(filePlace(PANE, SHOT))) }
        waitUntilAtLeastOneExists(hasContentDescription("shot.png, pinch to zoom, double tap to fit"), 5_000)
        assertEquals(png.size.toLong(), node.ranges.sumOf { it.last - it.first + 1 })
    }

    @Test
    fun aPictureThatWillNotDecodeSaysSoAndOffersTheDownload() = runComposeUiTest {
        val node = RangedNode(ByteArray(300_000) { 7 }, path = SHOT, mime = "image/png")
        setContent { Stage(node, Stack(filePlace(PANE, SHOT))) }
        waitUntilAtLeastOneExists(hasText("could not draw", substring = true), 5_000)
        onNodeWithContentDescription("Download shot.png to this device").assertExists()
    }

    @Test
    fun aFileNotHeldWholeDownloadsByStreamingFromTheNode() = runComposeUiTest {
        val node = RangedNode(WHOLE)
        setContent { Stage(node, Stack(filePlace(PANE, BIG))) }
        waitUntilAtLeastOneExists(hasContentDescription("Download big.log to this device"), 5_000)
        onNodeWithContentDescription("Download big.log to this device").performClick()
        waitUntilAtLeastOneExists(hasContentDescription("Saved to Downloads/big.log"), 5_000)
        assertEquals("big.log", node.downloaded)
        assertEquals(0, node.wholes)
    }
}
