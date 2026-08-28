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
import dev.kampr.shared.net.diffAttachmentId
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PATH = "/home/u/demo/notes.md"
private const val FILE_TEXT = "# notes\n\nthe fourth hop drops it\n"
private const val PATCH = "@@ -1,1 +1,2 @@\n-old line\n+the fourth hop drops it\n"

// A `Read` card as the node serves one: `summary` is filled from the tool's own `file_path`, so
// this is a path the node derived rather than one guessed at inside a sentence.
private const val READ_CARD = """{"t":"convo","pane":"01JNODE.../w3:p2","cursor":"t1","more":false,"turns":[
    {"id":"t1","role":"assistant","blocks":[
      {"b":"tool","name":"Read","summary":"/home/u/demo/notes.md","lines":3,"state":"done"}]}]}"""

private const val PROSE_CARD = """{"t":"convo","pane":"01JNODE.../w3:p2","cursor":"t1","more":false,"turns":[
    {"id":"t1","role":"assistant","blocks":[
      {"b":"tool","name":"Bash","summary":"list panes","lines":3,"state":"done"}]}]}"""

private fun paneOf(frame: String): PaneState {
    val store = KamprStore()
    store.accept(requireNotNull(Wire.decode(frame)) { "undecodable: $frame" })
    return store.pane(PANE_ID)
}

// The route answers `application/octet-stream` for anything that is not on its short list of image
// types, so what a text file comes back as is bytes with a media type that says nothing.
private fun node(readOnly: Boolean = false) = object : PaneIo {
    val asked = mutableListOf<Pair<String, String>>()
    override fun send(msg: dev.kampr.shared.wire.ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override val readOnly: Boolean = readOnly
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes {
        asked += paneId to id
        return when (id) {
            fileAttachmentId(PATH) ->
                AttachmentBytes.Ok(FILE_TEXT.encodeToByteArray(), "application/octet-stream")
            diffAttachmentId(PATH) -> AttachmentBytes.Ok(PATCH.encodeToByteArray(), "text/plain")
            else -> AttachmentBytes.Failed("no such attachment")
        }
    }
}

@Composable
private fun Screen(pane: PaneState, io: PaneIo) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides io,
    ) {
        Box(Modifier.fillMaxSize()) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) }
    }
}

// The node half of file retrieval was complete, tested, and minted by nothing in the client — the
// feature was built and unreachable. This is the half that reaches it.
@OptIn(ExperimentalTestApi::class)
class FileTargetSurfaceTest {
    @Test
    fun aToolCardNamingAFileOffersToFetchItAtTheIdTheRouteReads() = runComposeUiTest {
        val io = node()
        setContent { Screen(paneOf(READ_CARD), io) }
        onNodeWithContentDescription("Open notes.md").performClick()
        waitForIdle()
        assertEquals(listOf(PANE_ID to fileAttachmentId(PATH)), io.asked)
        onNodeWithText("the fourth hop drops it", substring = true).assertExists()
    }

    // A summary that is not a path is not a target. Detecting one inside prose is a guess about
    // English, and a guess that offers to fetch a file is worse than not offering.
    @Test
    fun aToolCardNamingNoFileOffersNothing() = runComposeUiTest {
        setContent { Screen(paneOf(PROSE_CARD), node()) }
        assertTrue(
            onAllNodesWithContentDescription("Open ", substring = true).fetchSemanticsNodes().isEmpty(),
            "a summary that was never a path was offered as one",
        )
    }

    // The whole security argument for a client-minted id: a device that may type into a terminal
    // can already `cat` the file, and a device that may not is exactly the one that must not reach
    // `~/.ssh/id_rsa`. The route refuses it outright, so the affordance is absent rather than
    // present-and-failing.
    @Test
    fun aReadOnlyDeviceIsOfferedNoFileAtAll() = runComposeUiTest {
        setContent { Screen(paneOf(READ_CARD), node(readOnly = true)) }
        assertTrue(
            onAllNodesWithContentDescription("Open notes.md").fetchSemanticsNodes().isEmpty(),
            "a read-only device was offered a file the node would refuse it",
        )
    }

    @Test
    fun theViewerOffersWhatGitSaysHasChangedInTheFileBesideIt() = runComposeUiTest {
        val io = node()
        setContent { Screen(paneOf(READ_CARD), io) }
        onNodeWithContentDescription("Open notes.md").performClick()
        waitForIdle()
        onNodeWithContentDescription("Show what has changed in notes.md since HEAD").performClick()
        waitForIdle()
        assertEquals(
            listOf(PANE_ID to fileAttachmentId(PATH), PANE_ID to diffAttachmentId(PATH)),
            io.asked,
        )
        onNodeWithText("+the fourth hop drops it", substring = true).assertExists()
    }
}
