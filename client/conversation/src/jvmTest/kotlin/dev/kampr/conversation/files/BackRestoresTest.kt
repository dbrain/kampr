package dev.kampr.conversation.files

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasScrollToKeyAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToIndex
import androidx.compose.ui.test.performScrollToKey
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.test.waitUntilExactlyOneExists
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.AttachmentSlice
import dev.kampr.shared.net.commitAttachmentId
import dev.kampr.shared.net.diffAttachmentId
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.net.lsAttachmentId
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import kotlinx.coroutines.CompletableDeferred
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val NODE = "01JNODE"
private const val ANCHOR = "01JNODE/w3:p2"
private const val DIR = "/home/u/dev/big"

private val NAMES = (0 until 60).map { "note-%02d.txt".format(it) } + "a-other.txt" + ".env"

private val BIG = """{"path":"$DIR","parent":"/home/u/dev","home":"/home/u","entries":[""" +
    NAMES.joinToString(",") { """{"name":"$it","kind":"file","bytes":10,"mtime":1730000000,"git":null}""" } +
    """],"truncated":false,"repo":null}"""

private fun row(name: String) = SemanticsMatcher("the row for $name") { node ->
    node.config.getOrNull(SemanticsProperties.ContentDescription).orEmpty().any { it.startsWith("$name,") }
}

// Every listing after the first waits for the test to let it through, so a return that showed a
// skeleton until the node answered again would be caught holding one.
private class SlowAgain : PaneIo {
    val again = CompletableDeferred<Unit>()
    var listed = 0
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes = when (id) {
        lsAttachmentId(DIR) -> {
            if (listed++ > 0) again.await()
            AttachmentBytes.Ok(BIG.encodeToByteArray(), "application/json")
        }

        fileAttachmentId("$DIR/note-40.txt") -> text("forty")

        else -> AttachmentBytes.Failed("no such attachment")
    }
}

private const val LOG = "/home/u/dev/big/run.log"

private val LOG_BYTES = (1..3000).joinToString("\n", postfix = "\n") { "row $it" }.encodeToByteArray()

private class Sliced(
    private val path: String = LOG,
    private val bytes: ByteArray = LOG_BYTES,
    private val answers: Map<String, String> = emptyMap(),
) : PaneIo {
    val ranges = mutableListOf<LongRange>()
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes = answers[id]?.let(::text) ?: AttachmentBytes.Failed("no such attachment")
    override suspend fun range(paneId: String, id: String, start: Long, endInclusive: Long): AttachmentSlice {
        if (id != fileAttachmentId(path)) return AttachmentSlice.Failed("no such attachment")
        val end = minOf(endInclusive, start + 4096 - 1, bytes.size - 1L)
        ranges += start..end
        return AttachmentSlice.Ok(bytes.copyOfRange(start.toInt(), end.toInt() + 1), "text/plain", bytes.size.toLong())
    }
}

// The report: filter a folder, scroll, open a file, go back — and the folder came back as if it
// had never been seen, with no filter, at the top, and a skeleton while it fetched itself again.
@OptIn(ExperimentalTestApi::class)
class BackRestoresTest {
    @Test
    fun backToAFolderRestoresItsFilterDotfilesAndScrollWithoutASkeleton() = runComposeUiTest {
        val io = SlowAgain()
        val nav = Stack(FilesPlace.Dir(NODE, ANCHOR, DIR))
        setContent { Stage(io, nav) }
        waitUntilExactlyOneExists(row("note-00.txt"))

        onNodeWithContentDescription("Show dotfiles").performClick()
        onNodeWithContentDescription("Search this folder and below").performTextInput("note")
        waitForIdle()
        assertTrue(onAllNodes(row("a-other.txt")).fetchSemanticsNodes().isEmpty())
        onNode(hasScrollToKeyAction()).performScrollToKey("note-40.txt")
        waitForIdle()
        assertTrue(onAllNodes(row("note-00.txt")).fetchSemanticsNodes().isEmpty(), "scrolled past the top")

        onNode(row("note-40.txt")).performClick()
        waitForIdle()
        assertEquals(2, nav.places.size, "the file opened over the folder")

        nav.back()
        waitForIdle()

        assertEquals(2, io.listed, "the folder asks again for what may have changed")
        onNode(row("note-40.txt")).assertExists()
        assertTrue(
            onAllNodes(row("note-00.txt")).fetchSemanticsNodes().isEmpty(),
            "the folder came back at the top",
        )
        onNodeWithContentDescription("Hide dotfiles").assertExists()

        onNode(hasScrollToKeyAction()).performScrollToKey("filter")
        waitForIdle()
        val filter = onNodeWithContentDescription("Search this folder and below").fetchSemanticsNode()
        assertEquals("note", filter.config[SemanticsProperties.EditableText].text)
        onNode(row("note-00.txt")).assertExists()
        assertTrue(onAllNodes(row("a-other.txt")).fetchSemanticsNodes().isEmpty(), "the filter is still applied")
        io.again.complete(Unit)
    }

    @Test
    fun aFolderOpenedAfreshAfterItWasLeftStartsFresh() = runComposeUiTest {
        val io = SlowAgain()
        io.again.complete(Unit)
        val place = FilesPlace.Dir(NODE, ANCHOR, DIR)
        val nav = Stack(place)
        setContent { Stage(io, nav) }
        waitUntilExactlyOneExists(row("note-00.txt"))
        onNodeWithContentDescription("Search this folder and below").performTextInput("note")
        waitForIdle()

        nav.back()
        waitForIdle()
        nav.open(place)
        waitUntilExactlyOneExists(row("note-00.txt"))

        val filter = onNodeWithContentDescription("Search this folder and below").fetchSemanticsNode()
        assertEquals("", filter.config[SemanticsProperties.EditableText].text)
        onNode(row("a-other.txt")).assertExists()
    }

    @Test
    fun backToAFileKeepsItsStreamAndWhereItWasRead() = runComposeUiTest {
        val io = Sliced()
        val nav = Stack(FilesPlace.File(NODE, ANCHOR, LOG))
        setContent { Stage(io, nav) }
        waitUntilExactlyOneExists(hasText("row 1", substring = false))
        val lines = SemanticsMatcher.keyIsDefined(SemanticsProperties.IndexForKey)
        repeat(20) {
            val read = onNode(lines).fetchSemanticsNode().config[SemanticsProperties.CollectionInfo].rowCount
            if (read > 1000) return@repeat
            onNode(lines).performScrollToIndex(read - 1)
            waitForIdle()
        }
        onNode(lines).performScrollToKey(899)
        waitForIdle()
        onNodeWithText("row 900").assertExists()
        val read = io.ranges.size

        onNodeWithContentDescription("Show the folder holding run.log").performClick()
        waitForIdle()
        nav.back()
        waitForIdle()

        assertEquals(read, io.ranges.size, "the file read itself again from the top")
        onNodeWithText("row 900").assertExists()
        assertTrue(onAllNodesWithText("row 1").fetchSemanticsNodes().isEmpty(), "the file came back at its top")
    }

    @Test
    fun backToAFileKeepsTheTabTheReadingAndTheDiffItWasOn() = runComposeUiTest {
        val notes = "/home/u/dev/big/notes.md"
        val nav = Stack(FilesPlace.File(NODE, ANCHOR, notes))
        val diff = "diff --git a/notes.md b/notes.md\n--- a/notes.md\n+++ b/notes.md\n@@ -1 +1 @@\n-# Old\n+# Notes\n"
        setContent { Stage(Sliced(notes, "# Notes\n\nsome words\n".encodeToByteArray(), mapOf(diffAttachmentId(notes) to diff)), nav) }
        waitUntilExactlyOneExists(hasContentDescription("Source view"))
        fun roundTrip() {
            onNodeWithContentDescription("Show the folder holding notes.md").performClick()
            waitForIdle()
            nav.back()
            waitForIdle()
        }

        onNodeWithContentDescription("Source view").performClick()
        roundTrip()
        onNodeWithContentDescription("Source view").assertIsSelected()

        onNodeWithContentDescription("Changes +1 −1 tab").performClick()
        waitForIdle()
        onNodeWithContentDescription("Side by side diff").performClick()
        roundTrip()
        onNodeWithContentDescription("Changes +1 −1 tab").assertIsSelected()
        onNodeWithContentDescription("Side by side diff").assertIsSelected()
    }

    @Test
    fun backToACommitKeepsThePatchesThatWereUnfolded() = runComposeUiTest {
        val repo = "/home/u/dev/big"
        val commit = "commit 8a50ea5\nAuthor: Danny <d@x>\n\n    many files\n\n" +
            (1..8).joinToString("") { "diff --git a/f$it.kt b/f$it.kt\n--- a/f$it.kt\n+++ b/f$it.kt\n@@ -1 +1 @@\n-old\n+new\n" }
        val nav = Stack(FilesPlace.Commit(NODE, ANCHOR, repo, "8a50ea5"))
        setContent { Stage(Sliced(answers = mapOf(commitAttachmentId(repo, "8a50ea5") to commit)), nav) }
        waitUntilExactlyOneExists(hasContentDescription("Show the patch for f1.kt"))
        onNodeWithContentDescription("Show the patch for f1.kt").performClick()
        onNodeWithContentDescription("Open f1.kt").performClick()
        waitForIdle()
        assertEquals(2, nav.places.size)

        nav.back()
        waitForIdle()
        onNodeWithContentDescription("Fold the patch for f1.kt").assertExists()
    }
}
