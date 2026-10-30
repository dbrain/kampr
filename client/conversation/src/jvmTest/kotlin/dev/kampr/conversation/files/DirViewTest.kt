package dev.kampr.conversation.files

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.waitUntilExactlyOneExists
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.tokensFor
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.UploadResult
import dev.kampr.shared.net.UploadSource
import dev.kampr.shared.net.lsAttachmentId
import dev.kampr.shared.net.zipAttachmentId
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.FilesNav
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val NODE = "01JNODE"
private const val ANCHOR = "01JNODE/w3:p2"
private const val DIR = "/home/u/dev/kampr"

private const val LISTING = """{"path":"$DIR","parent":"/home/u/dev","home":"/home/u","entries":[
  {"name":".git","kind":"dir","bytes":null,"mtime":1730000000,"git":null},
  {"name":"src","kind":"dir","bytes":null,"mtime":1730000000,"git":"M"},
  {"name":"README.md","kind":"file","bytes":4123,"mtime":1730000000,"git":"M"}],
  "truncated":false,
  "repo":{"root":"$DIR","branch":"main","head":"8a50ea5","upstream":"origin/main","ahead":0,"behind":2,"changed":3}}"""

// What the route answers for a path it will not serve, worded the way `AttachmentApi` words a 404.
private const val REFUSED = "The node no longer has this attachment."

private val UPLOADED = LISTING.replace(
    """{"name":"README.md"""",
    """{"name":"notes.txt","kind":"file","bytes":12,"mtime":1730000000,"git":"?"},
  {"name":"README.md"""",
)

private class Picked(override val name: String, override val size: Long?) : UploadSource {
    override suspend fun read(into: ByteArray): Int = -1
    override suspend fun close() = Unit
}

private class Node(
    private val uploaded: UploadResult = UploadResult.Ok("$DIR/notes.txt", 12),
    private val landed: String? = "Downloads/kampr.zip",
    override val readOnly: Boolean = false,
) : PaneIo {
    val asked = mutableListOf<Pair<String, String>>()
    val uploads = mutableListOf<Triple<String, String, String>>()
    val downloads = mutableListOf<Triple<String, String, String>>()
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes {
        asked += paneId to id
        val listing = if (uploads.isNotEmpty() && uploaded is UploadResult.Ok) UPLOADED else LISTING
        return if (id == lsAttachmentId(DIR)) AttachmentBytes.Ok(listing.encodeToByteArray(), "application/json") else AttachmentBytes.Failed(REFUSED, missing = true)
    }
    override suspend fun upload(paneId: String, dir: String, name: String, source: UploadSource, progress: (Long) -> Unit): UploadResult {
        uploads += Triple(paneId, dir, name)
        progress(6)
        return uploaded
    }
    override suspend fun download(paneId: String, id: String, name: String): String? {
        downloads += Triple(paneId, id, name)
        return landed
    }
}

private class Recorder : FilesNav {
    val opened = mutableListOf<FilesPlace>()
    override fun open(place: FilesPlace) {
        opened += place
    }
    override fun replace(place: FilesPlace) = Unit
    override fun back() = Unit
    override fun close() = Unit
}

private fun dir(path: String) = FilesPlace.Dir(NODE, ANCHOR, path)

@OptIn(ExperimentalTestApi::class)
class DirViewTest {
    private fun ComposeUiTest.show(
        place: FilesPlace.Dir,
        io: PaneIo = Node(),
        nav: FilesNav = Recorder(),
        picker: (suspend () -> UploadSource?)? = { Picked("notes.txt", 12) },
    ) {
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides io,
                LocalUploadPicker provides picker,
            ) {
                Box(Modifier.size(411.dp, 914.dp)) { DirView(place, nav) }
            }
        }
    }

    @Test
    fun everyWayOutOfAFolderOpensThePlaceItNames() = runComposeUiTest {
        val nav = Recorder()
        val io = Node()
        show(dir(DIR), io, nav)
        waitUntilExactlyOneExists(hasContentDescription("folder src", substring = true))
        assertEquals(listOf(ANCHOR to lsAttachmentId(DIR)), io.asked, "a listing is fetched through the pane it was opened from")

        onNodeWithContentDescription("folder src", substring = true).performClick()
        val readme = onNodeWithContentDescription("README.md, modified", substring = true)
        val spoken = readme.fetchSemanticsNode().config[SemanticsProperties.ContentDescription].joinToString()
        assertTrue(Regex("""4\.0 KB · \d+d""").containsMatchIn(spoken), "a file's age is read off the wall clock: $spoken")
        readme.performClick()
        onNodeWithContentDescription("Up to dev").performClick()
        onNodeWithContentDescription("Home folder").performClick()
        onNodeWithContentDescription("Git branch main", substring = true).performClick()

        assertEquals(
            listOf(
                dir("$DIR/src"),
                FilesPlace.File(NODE, ANCHOR, "$DIR/README.md"),
                dir("/home/u/dev"),
                dir("/home/u"),
                FilesPlace.Repo(NODE, ANCHOR, DIR),
            ),
            nav.opened,
        )
    }

    @Test
    fun dotfilesWaitForTheToggle() = runComposeUiTest {
        show(dir(DIR))
        waitUntilExactlyOneExists(hasContentDescription("folder src", substring = true))
        assertTrue(onAllNodesWithContentDescription("folder .git", substring = true).fetchSemanticsNodes().isEmpty())
        onNodeWithContentDescription("Show dotfiles").performClick()
        waitUntilExactlyOneExists(hasContentDescription("folder .git", substring = true))
    }

    @Test
    fun aRefusalSaysEverythingTheOneAnswerCouldMean() = runComposeUiTest {
        show(dir("/root"))
        waitUntilExactlyOneExists(hasText(NOT_BROWSABLE))
        onNodeWithText("Try again").performClick()
    }

    @Test
    fun anUploadLandsInTheListedFolderAndTheListingComesBackWithItMarked() = runComposeUiTest {
        val io = Node()
        show(dir(DIR), io)
        waitUntilExactlyOneExists(hasContentDescription("folder src", substring = true))
        onNodeWithContentDescription("Upload a file into kampr").performClick()
        waitUntilExactlyOneExists(hasText("Uploaded notes.txt (12 B)"))
        assertEquals(listOf(Triple(ANCHOR, DIR, "notes.txt")), io.uploads)
        assertEquals(2, io.asked.count { it.second == lsAttachmentId(DIR) }, "the folder is listed again once the file is in it")
        waitUntilExactlyOneExists(hasContentDescription("notes.txt, untracked", substring = true))
        onNodeWithContentDescription("just uploaded", substring = true).assertExists()
    }

    @Test
    fun aRefusedUploadSaysWhyAndLeavesTheListingAlone() = runComposeUiTest {
        val io = Node(uploaded = UploadResult.Failed("That folder is gone, or the node cannot write into it."))
        show(dir(DIR), io)
        waitUntilExactlyOneExists(hasContentDescription("folder src", substring = true))
        onNodeWithContentDescription("Upload a file into kampr").performClick()
        waitUntilExactlyOneExists(hasText("The upload did not land. That folder is gone, or the node cannot write into it."))
        assertEquals(1, io.asked.count { it.second == lsAttachmentId(DIR) })
    }

    @Test
    fun theFolderDownloadsAsOneZipNamedAfterIt() = runComposeUiTest {
        val io = Node()
        show(dir(DIR), io)
        waitUntilExactlyOneExists(hasContentDescription("folder src", substring = true))
        onNodeWithContentDescription("Download kampr as a zip").performClick()
        waitUntilExactlyOneExists(hasText("Saved kampr.zip to Downloads/kampr.zip"))
        assertEquals(listOf(Triple(ANCHOR, zipAttachmentId(DIR), "kampr.zip")), io.downloads)
    }

    @Test
    fun aZipThatDoesNotArriveSaysSo() = runComposeUiTest {
        show(dir(DIR), Node(landed = null))
        waitUntilExactlyOneExists(hasContentDescription("folder src", substring = true))
        onNodeWithContentDescription("Download kampr as a zip").performClick()
        waitUntilExactlyOneExists(hasText("kampr.zip did not arrive", substring = true))
    }

    @Test
    fun aReadOnlyDeviceIsOfferedNeitherAndNoPickerMeansNoUpload() = runComposeUiTest {
        var io by mutableStateOf(Node(readOnly = true))
        var picker by mutableStateOf<(suspend () -> UploadSource?)?>({ Picked("notes.txt", 12) })
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides io,
                LocalUploadPicker provides picker,
            ) {
                Box(Modifier.size(411.dp, 914.dp)) { DirView(dir(DIR), Recorder()) }
            }
        }
        waitUntilExactlyOneExists(hasContentDescription("folder src", substring = true))
        onNodeWithContentDescription("Upload a file into kampr").assertDoesNotExist()
        onNodeWithContentDescription("Download kampr as a zip").assertDoesNotExist()
        io = Node()
        picker = null
        waitUntilExactlyOneExists(hasContentDescription("Download kampr as a zip"))
        onNodeWithContentDescription("Upload a file into kampr").assertDoesNotExist()
    }
}
