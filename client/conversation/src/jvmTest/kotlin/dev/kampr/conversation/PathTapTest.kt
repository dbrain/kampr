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
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.LocalOpenPlace
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals

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
    private fun pressed(markdown: String, cwd: String?): Pair<List<FilesPlace>, List<String>> {
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
                        TurnView(Turn("a-1", "assistant", null, listOf(Block.Md(markdown))), "", emptyList(), {}, Modifier.fillMaxWidth())
                    }
                }
            }
            onNode(aLink()).performClick()
        }
        return places to uris
    }

    @Test
    fun aRelativePathInAReplyOpensTheFileInThePanesDirectory() {
        val (places, uris) = pressed("I changed `client/shared/Native.kt` for you.", "/home/u/dev/kampr")
        assertEquals(listOf<FilesPlace>(FilesPlace.File("01JNODE", PANE, "/home/u/dev/kampr/client/shared/Native.kt")), places)
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
}
