package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.conversation.files.HeldBytes
import dev.kampr.conversation.files.LocalHeldBytes
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.platform.PickedFile
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.LocalOpenPlace
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.ServerMsg
import kotlin.io.encoding.Base64
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

private val PICKED = PickedFile("shot.png", "image/png", byteArrayOf(1, 2, 3, 4, 5))
private const val PATH = "/home/u/.local/state/kampr/pastes/shot-1762061518000.png"
private const val REFUSED = "that is larger than this node will take"

private class Node(override val readOnly: Boolean = false, val pane: PaneState? = null) : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        if (pane != null) pane.noteUndelivered() else sent += msg
    }
    override fun prefs(paneId: String): PanePrefs = PanePrefs()

    val typed get() = sent.filterIsInstance<ClientMsg.InputText>().map { it.text }
    val pastes get() = sent.filterIsInstance<ClientMsg.Paste>()
}

private class Desk {
    val store = KamprStore()
    val pane = store.pane(PANE_ID)
    val held = HeldBytes()
    val opened = mutableListOf<FilesPlace>()
    val showing: MutableState<Boolean> = mutableStateOf(true)

    fun answer(path: String = PATH) {
        val id = pane.files.all.last().id
        store.accept(ServerMsg.Pasted(PANE_ID, id, path = path, text = "$path "))
    }
}

@Composable
private fun Screen(desk: Desk, io: PaneIo) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalConnectionStatus provides ConnectionStatus.Live("full"),
        LocalPaneIo provides io,
        LocalHeldBytes provides desk.held,
        LocalOpenPlace provides { place: FilesPlace -> desk.opened += place },
    ) {
        Box(Modifier.fillMaxSize()) {
            if (desk.showing.value) ConversationView(desk.pane, demoInfo(), Modifier.fillMaxSize())
        }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.send(words: String? = null) {
    words?.let { onNodeWithContentDescription("Reply to claude").performTextInput(it) }
    onNodeWithContentDescription("Send this reply to claude").performClick()
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.absent(description: String) = assertTrue(
    onAllNodesWithContentDescription(description).fetchSemanticsNodes().isEmpty(),
    "$description is still on the composer",
)

// A file handed to a conversation used to be typed into the pane as its path at once. That path
// sat on the harness's input line, came back as the desk line over the reply box, and a green line
// said so until the next send. The composer holds it as a chip now and the path goes with the reply.
@OptIn(ExperimentalTestApi::class)
class PasteSurfaceTest {
    @Test
    fun aFileGoesQuietlyAndRidesOnTheReplyAsItsPath() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }

        runOnIdle { attach(desk.pane, io, PICKED) }
        waitForIdle()
        val paste = io.pastes.single()
        assertEquals(ClientMsg.Paste(PANE_ID, Base64.encode(PICKED.bytes), "shot.png", typed = false, id = paste.id), paste)
        assertNotNull(paste.id)
        onNodeWithContentDescription("Sending shot.png").assertExists()
        onNodeWithContentDescription("Reply to claude").performTextInput("have a look at that")
        onNodeWithContentDescription("Send this reply to claude").assertIsNotEnabled()

        runOnIdle { desk.answer() }
        waitForIdle()
        onNodeWithContentDescription("Open shot.png").assertExists()
        assertTrue(io.typed.isEmpty(), "something was typed before the reply was sent: ${io.typed}")
        assertTrue(onAllNodesWithText("path is typed in", substring = true).fetchSemanticsNodes().isEmpty())

        send()
        assertEquals(listOf("$PATH have a look at that", "\r"), io.typed)
        absent("Open shot.png")
        assertTrue(desk.pane.files.all.isEmpty())
    }

    @Test
    fun aReplyOfNothingButFilesStillGoes() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }

        runOnIdle {
            attach(desk.pane, io, PICKED)
            desk.answer()
            attach(desk.pane, io, PickedFile("notes.txt", "text/plain", "hi".encodeToByteArray()))
            desk.answer("/p/notes-1.txt")
        }
        waitForIdle()
        onNodeWithContentDescription("Send this reply to claude").assertIsEnabled()
        send()
        assertEquals(listOf("$PATH /p/notes-1.txt", "\r"), io.typed)
    }

    @Test
    fun aRemovedChipTakesItsPathWithIt() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }

        runOnIdle {
            attach(desk.pane, io, PICKED)
            desk.answer()
        }
        waitForIdle()
        onNodeWithContentDescription("Remove shot.png").performClick()
        waitForIdle()
        absent("Open shot.png")
        send("never mind")
        assertEquals(listOf("never mind", "\r"), io.typed)
    }

    // The node's refusal is its only report that a file never arrived, so it stays until it is
    // read and dismissed, and a send that had nothing to do with it does not carry it or clear it.
    @Test
    fun aRefusalSaysWhyOnTheChipOutlivesASendAndIsDismissed() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }

        runOnIdle {
            attach(desk.pane, io, PICKED)
            desk.store.accept(ServerMsg.Pasted(PANE_ID, desk.pane.files.all.single().id, refused = REFUSED))
        }
        waitForIdle()
        onNodeWithText("shot.png: $REFUSED").assertExists()
        send("never mind")
        assertEquals(listOf("never mind", "\r"), io.typed)
        onNodeWithText("shot.png: $REFUSED").assertExists()

        onNodeWithContentDescription("Remove shot.png").performClick()
        waitForIdle()
        assertTrue(onAllNodesWithText(REFUSED, substring = true).fetchSemanticsNodes().isEmpty())
    }

    @Test
    fun aPaneErrorWhileAFileIsOnItsWayIsThatFilesRefusal() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }

        runOnIdle {
            attach(desk.pane, io, PICKED)
            desk.store.accept(ServerMsg.Failure("node_offline", "the node serving this pane left the herd", PANE_ID))
        }
        waitForIdle()
        onNodeWithText("shot.png: the node serving this pane left the herd").assertExists()
    }

    @Test
    fun aFileOverTheCeilingIsRefusedWithoutBeingSent() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }

        runOnIdle { attach(desk.pane, io, PickedFile("dump.bin", null, ByteArray(MOST_BYTES_HANDED_OVER + 1))) }
        waitForIdle()
        assertTrue(io.sent.isEmpty(), "the bytes went anyway")
        onNodeWithText("dump.bin: larger than the 8 MiB a node will take").assertExists()
    }

    // Dropped rather than queued for the reason a keystroke is, and a chip still saying "sending"
    // over bytes that were dropped is the lie that drop exists to prevent.
    @Test
    fun aFileWithNoSocketToGoDownIsRefusedNotSending() = runComposeUiTest {
        val desk = Desk()
        val io = Node(pane = desk.pane)
        setContent { Screen(desk, io) }

        runOnIdle { attach(desk.pane, io, PICKED) }
        waitForIdle()
        onNodeWithText("shot.png: not sent, there is no connection to the node").assertExists()
        absent("Sending shot.png")
    }

    @Test
    fun aChipOpensItsFileFromTheBytesThisDeviceAlreadyHas() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }

        runOnIdle {
            attach(desk.pane, io, PICKED)
            desk.answer()
        }
        waitForIdle()
        onNodeWithContentDescription("Open shot.png").performClick()
        waitForIdle()

        val place = assertIs<FilesPlace.Record>(desk.opened.single())
        assertEquals(PANE_ID, place.anchor)
        assertEquals("shot.png", place.att.name)
        assertEquals(PICKED.bytes.toList(), desk.held.get(PANE_ID, place.att.id)?.bytes?.toList())
    }

    // The composer leaves the composition whenever the terminal is looked at, and a file the node
    // has already written must not be lost to that glance.
    @Test
    fun aChipSurvivesTheComposerLeavingAndComingBack() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }

        runOnIdle { attach(desk.pane, io, PICKED) }
        waitForIdle()
        runOnIdle { desk.showing.value = false }
        waitForIdle()
        runOnIdle {
            desk.answer()
            desk.showing.value = true
        }
        waitForIdle()
        onNodeWithContentDescription("Open shot.png").assertExists()
    }

    @Test
    fun theComposerOffersToAttachSomethingWhereThereIsAPickerToRaise() = runComposeUiTest {
        setContent { Screen(Desk(), Node()) }
        onNodeWithContentDescription("Attach a file for claude").assertExists()
    }

    @Test
    fun aReadOnlyDeviceIsOfferedNothingToAttach() = runComposeUiTest {
        setContent { Screen(Desk(), Node(readOnly = true)) }
        absent("Attach a file for claude")
    }
}
