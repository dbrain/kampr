package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val SHOT_PATH = "/home/u/.kampr/paste/kampr-3f2.png"
private const val NOTES_PATH = "/home/u/demo/notes.md"
private const val SAID = "have a look at $SHOT_PATH"

// What the node leaves in the transcript when the operator attaches a picture: it writes the bytes
// on the pane's own machine and *types the path in*, so the turn is a path string and nothing in it
// says a picture was ever handed over.
private fun paneNaming(text: String, role: String = "user"): PaneState {
    val store = KamprStore()
    store.accept(
        ServerMsg.Convo(
            pane = PANE_ID, cursor = "u-1", more = false,
            turns = listOf(Turn("u-1", role, "2026-08-24T09:00:00.000Z", listOf(Block.Md(text)))),
        ),
    )
    return store.pane(PANE_ID)
}

private class Machine(
    private val answers: Map<String, AttachmentBytes> = emptyMap(),
    override val readOnly: Boolean = false,
) : PaneIo {
    val asked = mutableListOf<Pair<String, String>>()
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes {
        asked += paneId to id
        return answers[id] ?: AttachmentBytes.Failed("The file is no longer on that machine.")
    }
}

private fun withPicture() = Machine(
    mapOf(fileAttachmentId(SHOT_PATH) to AttachmentBytes.Ok(pngBytes(320, 200), "image/png")),
)

@Composable
private fun Screen(pane: PaneState, io: PaneIo) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides io,
    ) {
        Box(Modifier.fillMaxSize()) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) }
    }
}

@OptIn(ExperimentalTestApi::class)
class InlinePictureTest {
    // The ask: show it inline, in context, rather than leaving the reader a path they cannot open.
    @Test
    fun a_picture_the_operator_handed_over_is_shown_where_they_named_it() = runComposeUiTest {
        val io = withPicture()
        setContent { Screen(paneNaming(SAID), io) }
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription("Open kampr-3f2.png").fetchSemanticsNodes().isNotEmpty()
        }
        assertEquals(listOf(PANE_ID to fileAttachmentId(SHOT_PATH)), io.asked)
        onNodeWithText(SAID, substring = true).assertExists()
    }

    // Panning and zooming live in the one viewer that has them, not in a second one written for a
    // row inside a scrolling list.
    @Test
    fun pressing_it_opens_the_viewer_that_pans_and_zooms() = runComposeUiTest {
        setContent { Screen(paneNaming(SAID), withPicture()) }
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription("Open kampr-3f2.png").fetchSemanticsNodes().isNotEmpty()
        }
        onNodeWithContentDescription("Open kampr-3f2.png").performClick()
        waitForIdle()
        onNodeWithContentDescription(
            "kampr-3f2.png, pinch to zoom, double tap to fit",
            substring = true,
        ).assertExists()
    }

    // Narrow on purpose. `filePathOf` refuses to search prose for paths at all, and the only thing
    // that makes this defensible is that a token has to be a path *and* end in a type the node
    // serves inline before anything is fetched.
    @Test
    fun a_path_that_is_not_a_picture_is_the_text_it_always_was() = runComposeUiTest {
        val io = Machine()
        setContent { Screen(paneNaming("see $NOTES_PATH"), io) }
        waitForIdle()
        assertTrue(io.asked.isEmpty(), "a text file was fetched into the transcript unasked")
        onNodeWithText("see $NOTES_PATH", substring = true).assertExists()
    }

    // The agent naming a path is not the operator handing one over, and an agent that mentions
    // forty screenshots must not turn the transcript into forty fetches.
    @Test
    fun only_the_operators_own_turns_pull_a_picture_in() = runComposeUiTest {
        val io = withPicture()
        setContent { Screen(paneNaming(SAID, role = "assistant"), io) }
        waitForIdle()
        assertTrue(io.asked.isEmpty(), "a path inside an agent's reply was fetched unasked")
        onNodeWithText(SAID, substring = true).assertExists()
    }

    // The whole security argument for a path-shaped id: a device that may type into a terminal can
    // already `cat` the file, and a device that may not is exactly the one that must not reach
    // `~/.ssh/id_rsa`. It gets neither the fetch nor the affordance.
    @Test
    fun a_read_only_device_asks_for_nothing_and_is_offered_nothing() = runComposeUiTest {
        val io = Machine(readOnly = true)
        setContent { Screen(paneNaming(SAID), io) }
        waitForIdle()
        assertTrue(io.asked.isEmpty(), "a read-only device fetched a file the route would refuse it")
        assertTrue(
            onAllNodesWithContentDescription("Open kampr-3f2.png").fetchSemanticsNodes().isEmpty(),
            "a read-only device was offered a picture it cannot have",
        )
        onNodeWithText(SAID, substring = true).assertExists()
    }

    // The path resolves on the pane's machine at read time, so it renders only while the file is
    // still there. What it must never become is an empty frame that says nothing (#233).
    @Test
    fun a_fetch_that_fails_leaves_the_path_and_says_why() = runComposeUiTest {
        setContent { Screen(paneNaming(SAID), Machine()) }
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription("The file is no longer on that machine.")
                .fetchSemanticsNodes().isNotEmpty()
        }
        onNodeWithText(SAID, substring = true).assertExists()
    }

    // Each one is an authorised fetch and, decoded, several megabytes of pixels — the store holds
    // four across the whole pane.
    @Test
    fun one_message_cannot_pull_in_more_pictures_than_the_store_will_hold() {
        val said = (1..5).joinToString(" ") { "/home/u/shots/$it.png" }
        assertEquals(
            listOf("1.png", "2.png"),
            picturesIn(said).map { it.name },
        )
        assertTrue(picturesIn("no paths in this sentence at all").isEmpty())
        assertTrue(picturesIn("relative/shot.png is not one either").isEmpty())
        assertEquals(listOf("shot.png"), picturesIn("`~/shot.png`").map { it.name })
    }
}
