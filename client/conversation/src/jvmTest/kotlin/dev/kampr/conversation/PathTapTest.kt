package dev.kampr.conversation

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.conversation.files.FileNode
import dev.kampr.conversation.files.text
import dev.kampr.shared.model.Lookup
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.net.findAttachmentId
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.LocalOpenPlace
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.locate
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull

private const val PANE = "01JNODE/w1:p1"

@OptIn(ExperimentalTestApi::class)
private fun aLink() = SemanticsMatcher("a link inside prose") { node ->
    node.config.contains(SemanticsActions.OnClick) &&
        node.config.getOrNull(SemanticsProperties.Text) == null &&
        node.config.getOrNull(SemanticsProperties.ContentDescription) == null
}

// The operator's report: a path in the conversation could not be pressed — only a row under an
// expanded tool card opened anything. A path in the reply is a press that opens the file, resolved
// against the pane's own directory, and a URL beside it still leaves for the browser.
@OptIn(ExperimentalTestApi::class)
class PathTapTest {
    private fun pressed(markdown: String, cwd: String?): Pair<List<FilesPlace>, List<String>> = pressed(listOf(Block.Md(markdown)), cwd, 0)

    private fun pressed(blocks: List<Block>, cwd: String?, nth: Int): Pair<List<FilesPlace>, List<String>> {
        val places = mutableListOf<FilesPlace>()
        val uris = mutableListOf<String>()
        val browser = object : UriHandler {
            override fun openUri(uri: String) {
                uris += uri
            }
        }
        runComposeUiTest {
            setContent {
                CompositionLocalProvider(
                    LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                    LocalPaneIo provides RecordingIo,
                    LocalUriHandler provides browser,
                    LocalOpenPlace provides { places += it },
                ) {
                    PathLinks(PANE, cwd) {
                        TurnView(Turn("a-1", "assistant", null, blocks), "", emptyList(), {}, Modifier.fillMaxWidth())
                    }
                }
            }
            onAllNodes(aLink())[nth].performClick()
        }
        return places to uris
    }

    @Test
    fun aRelativePathInAReplyOpensTheFileInThePanesDirectory() {
        val (places, uris) = pressed("I changed `client/shared/Native.kt` for you.", "/home/u/dev/kampr")
        assertEquals(
            listOf<FilesPlace>(FilesPlace.File("01JNODE", PANE, "/home/u/dev/kampr/client/shared/Native.kt", Lookup("client/shared/Native.kt", "/home/u/dev/kampr", emptyList()))),
            places,
        )
        assertEquals(emptyList(), uris)
    }

    @Test
    fun anAbsolutePathInProseOpensAsWritten() {
        val (places, _) = pressed("wrote it to /tmp/out/report.md.", "/home/u")
        assertEquals(listOf<FilesPlace>(FilesPlace.File("01JNODE", PANE, "/tmp/out/report.md")), places)
    }

    @Test
    fun aUrlStillGoesToTheBrowser() {
        val (places, uris) = pressed("see https://kampr.dev/docs", "/home/u")
        assertEquals(emptyList(), places)
        assertEquals(listOf("https://kampr.dev/docs"), uris)
    }

    private fun resolved(place: FilesPlace, node: FileNode): String? {
        val file = place as FilesPlace.File
        return runBlocking { locate(node, file.anchor, assertNotNull(file.lookup, "a relative path was opened with nothing to look for")) }
    }

    // The operator's first report: from the repository root the agent wrote the path the way the
    // crate sees it, and the file is two directories down.
    @Test
    fun aPathWrittenFromBelowThePanesDirectoryOpensWhereItActuallyIs() {
        val (places, _) = pressed("Fixed it in `kampr-node/src/fs.rs`.", ROOT)
        val node = FileNode(mapOf(findAttachmentId(ROOT, "kampr-node/src/fs.rs") to found("$ROOT/crates/kampr-node/src/fs.rs")))
        assertEquals("$ROOT/crates/kampr-node/src/fs.rs", resolved(places.single(), node))
    }

    // The second: "look in <dir>", then a list of bare names. Each name is that directory's.
    @Test
    fun aBareNameListedAfterANamedDirectoryOpensInThatDirectory() {
        val markdown = "The parsers are in $SRC/\n\n- `parser.rs` — reads the frames\n- `lexer.rs` — splits them"
        val (places, _) = pressed(listOf(Block.Md(markdown)), ROOT, 1)
        val node = FileNode(mapOf(fileAttachmentId("$SRC/parser.rs") to text("x")))
        assertEquals("$SRC/parser.rs", resolved(places.single(), node))
    }

    // A reply is prose, a tool's output and prose again: a directory named before the code is
    // still the one a name after it was written against.
    @Test
    fun aDirectoryNamedEarlierInTheSameTurnCarriesAcrossItsOtherBlocks() {
        val blocks = listOf(Block.Md("Look in $SRC for the reader."), Block.Code("text", "ok"), Block.Md("Then open `src/lib.rs`."))
        val (places, _) = pressed(blocks, ROOT, 1)
        assertEquals(listOf(SRC), (places.single() as FilesPlace.File).lookup?.near)
    }

    @Test
    fun anAbsolutePathIsNeverLookedFor() {
        val (places, _) = pressed("Look in $SRC and then /tmp/out/report.md.", ROOT)
        assertNull((places.single() as FilesPlace.File).lookup)
    }
}

private const val ROOT = "/home/u/dev/kampr"
private const val SRC = "$ROOT/crates/kampr-node/src"

private fun found(vararg paths: String) = AttachmentBytes.Ok(
    paths.joinToString(",", """{"matches":[""", """],"truncated":false}""") { "\"$it\"" }.encodeToByteArray(),
    "application/json",
)
