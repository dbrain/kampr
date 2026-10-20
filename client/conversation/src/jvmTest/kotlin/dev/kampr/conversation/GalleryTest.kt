package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals

private val SHOTS = (1..8).map { "/tmp/shots/shot-$it.png" }

// What the node writes for a `SendUserFile` call naming eight screenshots.
private val SENT = Turn(
    id = "a-1",
    role = "assistant",
    at = null,
    blocks = listOf(Block.Md("Round 3.")) + SHOTS.map { Block.Md(it, fileTarget(it)) },
)

@OptIn(ExperimentalTestApi::class)
class GalleryTest {
    private val node = NodeWithAttachments(SHOTS.associate { fileAttachmentId(it) to AttachmentBytes.Ok(pngBytes(1080, 2400), "image/png") })

    // The operator, on a reply that sent eight: a different one of them failed every time they came
    // back to the pane. The store holds four full pictures, so eight inline ones were always
    // pushing each other out and fetching again.
    @Test
    fun eightSentPicturesAreEightNamedTilesThatStayAcrossALeaveAndAReturn() = runComposeUiTest {
        val store = AttachmentStore(PANE_ID)
        var here by mutableStateOf(true)
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides node,
            ) {
                if (here) {
                    Box(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                        TurnView(SENT, "", emptyList(), {}, Modifier.fillMaxWidth(), store)
                    }
                }
            }
        }
        val tiles = { onAllNodesWithContentDescription("Open shot-", substring = true).fetchSemanticsNodes().size }
        waitUntil(timeoutMillis = 10_000) { tiles() == 8 }
        for (n in 1..8) onNodeWithText("shot-$n.png").assertExists()

        here = false
        waitForIdle()
        here = true
        waitForIdle()
        assertEquals(8, tiles(), "a tile came back as something other than its picture")
        assertEquals(8, node.asked.size, "a tile fetched again: ${node.asked}")

        onNodeWithContentDescription("Open shot-6.png").performClick()
        waitUntil(timeoutMillis = 5_000) { store.viewing?.name == "shot-6.png" }
        assertEquals(AttachmentState.Shown::class, store.state(fileAttachmentId(SHOTS[5]))::class)
    }
}
