package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import androidx.compose.ui.test.performClick
import kotlin.test.assertEquals
import kotlin.test.assertTrue

@Composable
private fun Themed(content: @Composable () -> Unit) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
    ) {
        Box(Modifier.fillMaxSize()) { content() }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.targets(): List<String> =
    onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.Text))
        .fetchSemanticsNodes()
        .flatMap { it.config.getOrNull(SemanticsProperties.Text).orEmpty() }
        .flatMap { text ->
            text.getLinkAnnotations(0, text.length).mapNotNull { (it.item as? LinkAnnotation.Url)?.url }
        }

// Compose gives every link inside a paragraph its own clickable node with no text and no label of
// its own, which is what tells it apart from every button around it.
@OptIn(ExperimentalTestApi::class)
private fun theLink() = SemanticsMatcher("a link inside prose") { node ->
    node.config.contains(SemanticsActions.OnClick) &&
        node.config.getOrNull(SemanticsProperties.Text) == null &&
        node.config.getOrNull(SemanticsProperties.ContentDescription) == null
}

private class Opened : UriHandler {
    val uris = mutableListOf<String>()
    override fun openUri(uri: String) { uris += uri }
}

// A URL a reader can see is a URL they should be able to press, and the surfaces that are not
// markdown — tool output, a fenced block, a patch — are where most of them actually appear.
@OptIn(ExperimentalTestApi::class)
class LinkSurfaceTest {
    @Test
    fun aUrlInProseIsPressableWhereTheReaderSeesIt() = runComposeUiTest {
        val turn = Turn("a-1", "assistant", null, listOf(Block.Md("the cause is at https://kampr.dev/docs#e17.")))
        setContent { Themed { TurnView(turn, "", emptyList(), {}, Modifier.fillMaxWidth()) } }
        assertEquals(listOf("https://kampr.dev/docs#e17"), targets())
    }

    @Test
    fun aUrlInToolOutputIsPressableWhereTheReaderSeesIt() = runComposeUiTest {
        val turn = Turn(
            "a-2", "assistant", null,
            listOf(
                Block.Tool("Bash", "curl the docs", 2, "done"),
                Block.Code("bash", "curl -sS https://kampr.dev/docs/probe.json\n# 404 — see https://kampr.dev/help"),
            ),
        )
        setContent { Themed { TurnView(turn, "", listOf("a-2#0"), {}, Modifier.fillMaxWidth()) } }
        assertEquals(
            listOf("https://kampr.dev/docs/probe.json", "https://kampr.dev/help"),
            targets(),
        )
    }

    @Test
    fun aUrlInAPatchIsPressableWhereTheReaderSeesIt() = runComposeUiTest {
        val turn = Turn(
            "a-3", "assistant", null,
            listOf(Block.Diff("README.md", "@@ -1 +1 @@\n-see http://old.example/x\n+see https://new.example/x")),
        )
        setContent { Themed { TurnView(turn, "", emptyList(), {}, Modifier.fillMaxWidth()) } }
        assertEquals(listOf("http://old.example/x", "https://new.example/x"), targets())
    }

    @Test
    fun pressingABareUrlOpensIt() = runComposeUiTest {
        val opened = Opened()
        val turn = Turn("a-5", "assistant", null, listOf(Block.Md("the cause is at https://kampr.dev/docs and there")))
        setContent {
            CompositionLocalProvider(LocalUriHandler provides opened) {
                Themed { TurnView(turn, "", emptyList(), {}, Modifier.fillMaxWidth()) }
            }
        }
        onNode(theLink()).performClick()
        assertEquals(listOf("https://kampr.dev/docs"), opened.uris)
    }

    // A URL is one unbroken run of characters and a phone column is 358 dp of it. There is no
    // space to break on, so a paragraph that will not break inside a word measures as wide as the
    // URL and takes the turn beside it off the side of the transcript. Measured off the layout
    // rather than off the node, because the node is clipped to its parent either way.
    @Test
    fun aVeryLongUrlIsBrokenToFitTheColumnItWasGiven() = runComposeUiTest {
        val long = "https://kampr.dev/" + "a".repeat(300)
        val turn = Turn("a-6", "assistant", null, listOf(Block.Md("look at $long now")))
        setContent {
            Themed { Box(Modifier.width(358.dp)) { TurnView(turn, "", emptyList(), {}, Modifier.fillMaxWidth()) } }
        }
        val laid = mutableListOf<TextLayoutResult>()
        onNodeWithText("look at", substring = true)
            .fetchSemanticsNode()
            .config[SemanticsActions.GetTextLayoutResult]
            .action!!
            .invoke(laid)
        val layout = laid.single()
        assertTrue(layout.lineCount > 1, "a 300-character URL was laid out on one line")
        assertTrue(
            layout.size.width <= 358 * 2,
            "the paragraph measured ${layout.size.width} px inside a 358 dp column",
        )
    }
}
