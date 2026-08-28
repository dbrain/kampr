package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.platform.PickedFile
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.file.Handover
import dev.kampr.terminal.file.MOST_BYTES_HANDED_OVER
import dev.kampr.terminal.file.handoverAfter
import dev.kampr.terminal.file.handoverOf
import dev.kampr.terminal.view.HandoverLine
import kotlinx.coroutines.runBlocking
import kotlin.io.encoding.Base64
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertTrue

private val SHOT = PickedFile("shot.png", "image/png", byteArrayOf(1, 2, 3, 4, 5))

private class Node(override val readOnly: Boolean = false) : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
}

private fun storedPane(): Pair<KamprStore, PaneState> {
    val store = KamprStore()
    return store to store.pane(Phone.PANE)
}

// Paste existed only in the conversation composer, and the terminal is where an operator watching
// `claude` actually is: there was no way to put a screenshot in front of it from the surface they
// were already looking at.
@OptIn(ExperimentalTestApi::class)
class HandingAFileToAPaneTest {
    @Test
    fun theBytesGoAsBase64AndTheNameGoesAsAHintAtTheStem() = runBlocking {
        val (_, pane) = storedPane()
        val io = Node()
        val handover = handoverOf(pane, io, SHOT)
        assertEquals(
            listOf(ClientMsg.Paste(Phone.PANE, Base64.encode(SHOT.bytes), "shot.png")),
            io.sent.filterIsInstance<ClientMsg.Paste>(),
        )
        assertIs<Handover.Sent>(handover)
        Unit
    }

    @Test
    fun aFileOverTheCeilingIsRefusedWithoutBeingSent() = runBlocking {
        val (_, pane) = storedPane()
        val io = Node()
        val handover = handoverOf(pane, io, PickedFile("dump.bin", null, ByteArray(MOST_BYTES_HANDED_OVER + 1)))
        assertTrue(io.sent.isEmpty(), "the bytes went anyway")
        assertIs<Handover.Refused>(handover)
        Unit
    }

    @Test
    fun aRefusalFromTheNodeReplacesTheSentLine() {
        assertEquals(
            Handover.Refused("that is larger than this node will take"),
            handoverAfter(Handover.Sent("shot.png"), "that is larger than this node will take"),
        )
        assertEquals(
            Handover.Idle,
            handoverAfter(Handover.Idle, "an answer key is one or two characters"),
            "a refusal about something else was charged to a paste nobody sent",
        )
    }

    @Test
    fun theTerminalOffersToAttachSomethingWhereThereIsAPickerToRaise() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(), session)
        onNodeWithContentDescription("Attach a file for this pane").assertExists()
    }

    @Test
    fun aReadOnlyDeviceIsOfferedNothingToAttach() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(), session, io = Node(readOnly = true))
        assertTrue(
            onAllNodesWithContentDescription("Attach a file for this pane").fetchSemanticsNodes().isEmpty(),
            "a device that cannot type was offered a way to type a path in",
        )
    }

    @Test
    fun whatIsGoingIsSaidOnTheTerminalsOwnChrome() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides Phone.tokens()) {
                Box(Modifier.fillMaxSize()) { HandoverLine(Handover.Going("shot.png")) }
            }
        }
        onNodeWithText("sending shot.png").assertExists()
    }

    @Test
    fun aRefusedPasteSaysWhyOnTheTerminalsOwnChrome() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides Phone.tokens()) {
                Box(Modifier.fillMaxSize()) {
                    HandoverLine(Handover.Refused("that is larger than this node will take"))
                }
            }
        }
        onNodeWithText("that is larger than this node will take").assertExists()
    }

    // The line is rendered by the terminal's own chrome, not only by a composable a test can call:
    // removing the call site left every strip assertion above green and the operator with nothing
    // on screen. This is the one that fails for that.
    @Test
    fun theTerminalSaysWhatWentAndThenSaysWhatTheNodeRefused() = runComposeUiTest {
        val (store, pane) = storedPane()
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session, io = Node())
        session.handover = Handover.Sent("shot.png")
        waitForIdle()
        onNodeWithText("shot.png is on the pane's machine, and its path is typed in").assertExists()

        store.accept(ServerMsg.Failure("bad_request", "that is larger than this node will take", Phone.PANE))
        waitForIdle()
        onNodeWithText("that is larger than this node will take").assertExists()
    }

    // The store is what carries the node's refusal to the pane it is about, and the strip reads it
    // off the pane rather than off something floating over another screen.
    @Test
    fun theNodesRefusalReachesThePaneItNames() {
        val (store, pane) = storedPane()
        store.accept(ServerMsg.Failure("bad_request", "that is larger than this node will take", Phone.PANE))
        assertEquals("that is larger than this node will take", pane.refusal)
    }
}
