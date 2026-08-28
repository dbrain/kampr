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
import dev.kampr.shared.platform.PickedFile
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.ServerMsg
import kotlin.io.encoding.Base64
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertTrue

private val PICKED = PickedFile("shot.png", "image/png", byteArrayOf(1, 2, 3, 4, 5))

private class Node(override val readOnly: Boolean = false) : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    override fun send(msg: ClientMsg) {
        sent += msg
    }
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
}

private fun emptyPane(): Pair<KamprStore, PaneState> {
    val store = KamprStore()
    return store to store.pane(PANE_ID)
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

// There was no way to put a picture in front of an agent from a phone: the client could send
// `input.text`, `input.b64` and `input.keys` and nothing else. The bytes go to the node, which
// writes them beside the pane and types the path in — an agent over ssh reads a local path
// perfectly well, and it is the terminal's own image-paste protocol that dies.
@OptIn(ExperimentalTestApi::class)
class PasteSurfaceTest {
    @Test
    fun theBytesGoAsBase64AndTheNameGoesAsAHintAtTheStem() {
        val (_, pane) = emptyPane()
        val io = Node()
        val handover = handoverOf(pane, io, PICKED)
        assertEquals(
            listOf(ClientMsg.Paste(PANE_ID, Base64.encode(PICKED.bytes), "shot.png")),
            io.sent.filterIsInstance<ClientMsg.Paste>(),
        )
        assertIs<Handover.Sent>(handover)
    }

    // The node's own ceiling, applied here as well: sending eight megabytes up a phone link to be
    // refused at the other end is a minute of somebody's tethering spent on a certain no.
    @Test
    fun aFileOverTheCeilingIsRefusedWithoutBeingSent() {
        val (_, pane) = emptyPane()
        val io = Node()
        val huge = PickedFile("dump.bin", null, ByteArray(MOST_BYTES_HANDED_OVER + 1))
        val handover = handoverOf(pane, io, huge)
        assertTrue(io.sent.isEmpty(), "the bytes went anyway")
        assertIs<Handover.Refused>(handover)
    }

    // A paste the node will not take comes back as an error naming this pane, and that error is
    // quiet everywhere else by design — so this is the only place it can be said.
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
    fun theComposerOffersToAttachSomethingWhereThereIsAPickerToRaise() = runComposeUiTest {
        val (_, pane) = emptyPane()
        setContent { Screen(pane, Node()) }
        onNodeWithContentDescription("Attach a file for claude").assertExists()
    }

    @Test
    fun aReadOnlyDeviceIsOfferedNothingToAttach() = runComposeUiTest {
        val (_, pane) = emptyPane()
        setContent { Screen(pane, Node(readOnly = true)) }
        assertTrue(
            onAllNodesWithContentDescription("Attach a file for claude").fetchSemanticsNodes().isEmpty(),
            "a device that cannot type was offered a way to type a path in",
        )
    }

    @Test
    fun whatIsGoingAndWhatWasRefusedAreBothSaidOnTheComposer() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone)) {
                Box(Modifier.fillMaxSize()) {
                    Composer("claude", enabled = true, onSend = {}, handover = Handover.Going("shot.png"))
                }
            }
        }
        onNodeWithText("sending shot.png").assertExists()
    }

    @Test
    fun aRefusedPasteSaysWhyOnTheComposer() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone)) {
                Box(Modifier.fillMaxSize()) {
                    Composer(
                        "claude",
                        enabled = true,
                        onSend = {},
                        handover = Handover.Refused("that is larger than this node will take"),
                    )
                }
            }
        }
        onNodeWithText("that is larger than this node will take").assertExists()
    }

    // The store is what carries the node's refusal to the pane it is about, and the composer reads
    // it off the pane rather than off a strip floating over some other screen.
    @Test
    fun theNodesRefusalReachesThePaneItNames() {
        val (store, pane) = emptyPane()
        store.accept(
            ServerMsg.Failure("bad_request", "that is larger than this node will take", PANE_ID)
        )
        assertEquals("that is larger than this node will take", pane.refusal)
    }
}
