package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
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
import androidx.compose.ui.test.performTextInputSelection
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.text.TextRange
import dev.kampr.conversation.files.HeldBytes
import dev.kampr.conversation.files.LocalHeldBytes
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PLACED_ANSWER_WAIT_MS
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
import dev.kampr.shared.wire.EditKeys
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.PendingOption
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
private val KEYS = EditKeys(back = "\u007f", left = "\u001b[D", right = "\u001b[C", newline = "\n")
private val PICTURE_KEYS = KEYS.copy(images = true)

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
private fun ComposeUiTest.replyText(): String = onNodeWithContentDescription("Reply to claude").fetchSemanticsNode().config[SemanticsProperties.EditableText].text

// `alpha gamma` typed into a Claude pane's mirrored box, the caret put after `alpha `, a picture
// attached there, and the pane's line come back holding its token.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.placed(desk: Desk, io: Node) {
    val reply = onNodeWithContentDescription("Reply to claude")
    runOnIdle { desk.store.accept(ServerMsg.ConvoComposer(PANE_ID, "", "\u0003", 0, PICTURE_KEYS)) }
    waitForIdle()
    reply.performTextInput("alpha gamma")
    reply.performTextInputSelection(TextRange(6))
    waitForIdle()
    runOnIdle { attach(desk.pane, io, PICKED, live = true) }
    waitForIdle()

    assertEquals(listOf("alpha gamma", "\u001b[D".repeat(5)), io.typed)
    val paste = io.pastes.single()
    assertEquals(true, paste.typed)
    runOnIdle {
        desk.store.accept(ServerMsg.Pasted(PANE_ID, paste.id, path = PATH, text = "\u001b[200~$PATH\u001b[201~"))
        desk.store.accept(ServerMsg.ConvoComposer(PANE_ID, "alpha [Image #1]gamma", "\u0003", 16, PICTURE_KEYS))
    }
    waitUntil(timeoutMillis = 5_000) { replyText() == "alpha [Image #1]gamma" }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.absent(description: String) = assertTrue(
    onAllNodesWithContentDescription(description).fetchSemanticsNodes().isEmpty(),
    "$description is still on the composer",
)

// A file handed to a conversation used to be typed into the pane as its path at once. That path
// sat on the harness's input line, came back as the desk line over the reply box, and a green line
// said so until the next send. A box that is not the pane's line holds it as a label at its caret
// now, beside a chip, and the label becomes the file's path or paste when the reply goes.
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
        assertEquals("[image 1: shot.png] ", replyText())
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

    // The words were typed into the pane as they were written, so the files go after them and the
    // send types only what the pane's line lacks. Files in front made the box disagree with the
    // line, and the send took the line back and retyped it whole: Claude kept the Enter that
    // followed as a newline (#619).
    @Test
    fun aFileSentWithWordsAlreadyInThePaneIsTypedAfterThemWithNothingTakenBack() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }

        runOnIdle { desk.store.accept(ServerMsg.ConvoComposer(PANE_ID, "", "\u0003", 0, KEYS)) }
        waitForIdle()
        onNodeWithContentDescription("Reply to claude").performTextInput("have a look at that")
        waitForIdle()
        assertEquals(listOf("have a look at that"), io.typed)
        runOnIdle {
            attach(desk.pane, io, PICKED)
            desk.answer()
        }
        waitForIdle()

        send()
        assertEquals(listOf("have a look at that", " $PATH", "\r"), io.typed)
    }

    // Claude and Codex draw a picture pasted into their box as one `[Image #n]` and send the
    // picture itself (#620). So the file goes into the pane where the reply box's caret is, the
    // token comes back as the line, and the send is the Enter alone.
    @Test
    fun aPictureForAHarnessThatDrawsOneLandsAtTheCaretAndTheSendIsOnlyTheEnter() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }
        placed(desk, io)
        absent("Open shot.png")
        absent("Sending shot.png")

        send()
        assertEquals(listOf("alpha gamma", "\u001b[D".repeat(5), "\r"), io.typed)
    }

    // A node or a hub from before placed files were answered types the file and says nothing, so
    // the chip does not hold the reply back for ever: a failure would have come as a pane error.
    @Test
    fun aPlacedFileThatIsNeverAnsweredStopsHoldingTheReply() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }
        val reply = onNodeWithContentDescription("Reply to claude")
        runOnIdle { desk.store.accept(ServerMsg.ConvoComposer(PANE_ID, "", "\u0003", 0, PICTURE_KEYS)) }
        waitForIdle()
        reply.performTextInput("alpha")
        waitForIdle()
        runOnIdle { attach(desk.pane, io, PICKED, live = true) }
        waitForIdle()
        onNodeWithContentDescription("Send this reply to claude").assertIsNotEnabled()

        // In steps, so the wait is over however late in the first frames its effect began.
        repeat(((PLACED_ANSWER_WAIT_MS + 1_000) / 1_000).toInt() * 2) {
            mainClock.advanceTimeBy(1_000)
            waitForIdle()
        }
        absent("Sending shot.png")
        onNodeWithContentDescription("Send this reply to claude").assertIsEnabled()
    }

    // The harness takes the token whole with one key, so the reply box does too.
    @Test
    fun cuttingIntoAPlacedTokenTakesItWholeHereAndInThePane() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }
        placed(desk, io)

        onNodeWithContentDescription("Reply to claude").performTextReplacement("alpha [Image #1gamma")
        waitForIdle()
        assertEquals("alpha gamma", replyText())
        assertEquals("\u007f", io.typed.last())
    }

    // A box that went out of step while a question stood on the pane, holding a picture the pane
    // has since lost: the send puts the file back rather than typing `[Image #1]` as words.
    @Test
    fun aPictureThePaneLostWhileTheBoxWasApartGoesBackInAsTheFile() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }
        placed(desk, io)
        val asked = ServerMsg.Pending(PANE_ID, "Proceed?", listOf(PendingOption("1", "Yes")), "screen")

        runOnIdle { desk.store.accept(asked) }
        waitForIdle()
        onNodeWithContentDescription("Reply to claude").performTextInput("!")
        waitForIdle()
        runOnIdle {
            desk.store.accept(ServerMsg.ConvoComposer(PANE_ID, "alpha gamma", "\u0003", 6, PICTURE_KEYS))
            desk.store.accept(asked.copy(question = null))
        }
        waitForIdle()
        assertEquals("alpha [Image #1]!gamma", replyText())

        send()
        assertEquals(listOf("\u001b[200~$PATH\u001b[201~", "!", "\r"), io.typed.takeLast(3))
    }

    // The label goes where the caret is, set apart from the words either side, and an edit that
    // cuts into it takes it and its file whole.
    @Test
    fun aLabelLandsAtTheCaretAndCuttingIntoItTakesTheFile() = runComposeUiTest {
        val desk = Desk()
        val io = Node()
        setContent { Screen(desk, io) }
        val reply = onNodeWithContentDescription("Reply to claude")

        reply.performTextInput("alpha gamma")
        reply.performTextInputSelection(TextRange(5))
        waitForIdle()
        runOnIdle {
            attach(desk.pane, io, PICKED)
            desk.answer()
        }
        waitForIdle()
        assertEquals("alpha [image 1: shot.png] gamma", replyText())

        reply.performTextReplacement("alpha [image 1: shot.png gamma")
        waitForIdle()
        assertEquals("alpha  gamma", replyText())
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
        assertEquals("", replyText())
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
        assertEquals("", replyText())
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
