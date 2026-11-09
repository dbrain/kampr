package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.test.waitUntilAtLeastOneExists
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private const val CLIP = "/home/u/demo/clip.wav"
private const val NOTES = "/home/u/demo/notes.md"

// The shape the report arrived in: an agent produced audio with a `Bash` call — whose `summary` is
// the tool's `description`, not its output path — and then typed the paths into its reply. There
// is no `att` on any of it and no tool card that names it, so prose is the only thing the pane has.
private fun said(text: String, role: String = "assistant") = """{"t":"convo","pane":"$PANE_ID","cursor":"s1","more":false,"turns":[
        {"id":"s1","role":"$role","blocks":[{"b":"md","text":"$text"}]}]}"""

private val SPOKEN = said("rendered it to $CLIP \\u2014 my notes are in $NOTES")

private const val VOICE_NOTE = "att-voice"

private val ATTACHED = """{"t":"convo","pane":"$PANE_ID","cursor":"s1","more":false,"turns":[
        {"id":"s1","role":"user","blocks":[{"b":"md","text":"[audio]","att":{"id":"$VOICE_NOTE","kind":"audio","mime":"audio/wav","name":"memo.wav"}}]}]}"""

private const val TICKET = "https://node.test/api/stream/tkt_clip"

private class SoundNode(
    private val type: String? = "audio/wav",
    override val readOnly: Boolean = false,
) : PaneIo {
    val asked = mutableListOf<String>()
    val streamed = mutableListOf<String>()
    val downloaded = mutableListOf<Pair<String, String>>()

    override fun send(msg: ClientMsg) = Unit

    override fun prefs(paneId: String): PanePrefs = PanePrefs()

    override suspend fun attachment(paneId: String, id: String): AttachmentBytes {
        asked += id
        return if (id == fileAttachmentId(CLIP)) {
            AttachmentBytes.Ok(ByteArray(64) { it.toByte() }, type)
        } else {
            AttachmentBytes.Failed("no such attachment")
        }
    }

    override suspend fun streamUrl(paneId: String, id: String): String? {
        streamed += id
        return TICKET.takeIf { id == fileAttachmentId(CLIP) || id == VOICE_NOTE }
    }

    override suspend fun download(paneId: String, id: String, name: String): String {
        downloaded += id to name
        return "Downloads/$name"
    }
}

private fun paneOf(frame: String): PaneState {
    val store = KamprStore()
    store.accept(requireNotNull(Wire.decode(frame)) { "undecodable: $frame" })
    return store.pane(PANE_ID)
}

@Composable
private fun Screen(pane: PaneState, io: PaneIo, voices: Voices) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides io,
        LocalVoices provides voices,
    ) {
        Box(Modifier.fillMaxSize()) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) }
    }
}

@OptIn(ExperimentalTestApi::class)
class SoundSurfaceTest {
    @Test
    fun a_recording_an_agent_named_is_a_row_the_reader_can_press() = runComposeUiTest {
        val io = SoundNode()
        setContent { Screen(paneOf(SPOKEN), io, Decoder()) }
        waitForIdle()

        onNodeWithContentDescription("Play clip.wav").assertExists()
        assertTrue(io.asked.isEmpty(), "a recording was fetched into the transcript unasked")
        assertTrue(
            onAllNodesWithContentDescription("notes.md", substring = true).fetchSemanticsNodes().isEmpty(),
            "a path that names no recording was offered as one",
        )
    }

    // A recording is played from where it lies rather than fetched into memory first: one press
    // mints a stream at the id the route reads, the player is handed that URL and starts, and not a
    // byte of the file crosses into this client on the way.
    @Test
    fun one_press_streams_it_from_the_id_the_route_reads_and_plays() = runComposeUiTest {
        val io = SoundNode()
        val decoder = Decoder()
        setContent { Screen(paneOf(SPOKEN), io, decoder) }
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitUntilAtLeastOneExists(hasContentDescription("Pause clip.wav"), 5_000)

        assertEquals(listOf(fileAttachmentId(CLIP)), io.streamed)
        assertEquals(TICKET, decoder.latest?.url)
        assertTrue(decoder.latest?.playing == true, "the press reached no player")
        assertTrue(io.asked.isEmpty(), "the recording was fetched whole instead of streamed")
        onNodeWithContentDescription("Save clip.wav").assertExists()
    }

    @Test
    fun the_button_pauses_and_plays_the_same_sound() = runComposeUiTest {
        val decoder = Decoder()
        setContent { Screen(paneOf(SPOKEN), SoundNode(), decoder) }
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitUntilAtLeastOneExists(hasContentDescription("Pause clip.wav"), 5_000)

        onNodeWithContentDescription("Pause clip.wav").performClick()
        waitForIdle()
        assertFalse(decoder.latest?.playing == true, "pausing left the sound running")
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitForIdle()
        assertTrue(decoder.latest?.playing == true, "playing again did nothing")
        onNodeWithContentDescription("Pause clip.wav").assertExists()
    }

    // A sound that runs to its end is not paused by anybody, and the button has to come back on
    // its own or the reader is left looking at a pause that would do nothing.
    @Test
    fun a_sound_that_reaches_its_end_puts_its_own_button_back() = runComposeUiTest {
        val decoder = Decoder()
        setContent { Screen(paneOf(SPOKEN), SoundNode(), decoder) }
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitUntilAtLeastOneExists(hasContentDescription("Pause clip.wav"), 5_000)

        decoder.latest?.ended()
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription("Play clip.wav").fetchSemanticsNodes().isNotEmpty()
        }
    }

    // A type this device claims and a stream it then cannot read. The reader is told, and is still
    // given the file — which is the floor the operator asked for.
    @Test
    fun a_stream_the_decoder_will_not_take_says_so_and_leaves_the_file_on_offer() = runComposeUiTest {
        setContent { Screen(paneOf(SPOKEN), SoundNode(), Decoder(deaf = true)) }
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitUntilAtLeastOneExists(hasText("would not play", substring = true), 5_000)
        onNodeWithContentDescription("Save clip.wav").assertExists()
    }

    // Nothing of the recording is held, so keeping it streams from the node to the downloads.
    @Test
    fun saving_a_streamed_recording_downloads_it_from_the_node() = runComposeUiTest {
        val io = SoundNode()
        setContent { Screen(paneOf(SPOKEN), io, Decoder()) }
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitUntilAtLeastOneExists(hasContentDescription("Save clip.wav"), 5_000)
        onNodeWithContentDescription("Save clip.wav").performClick()
        waitUntilAtLeastOneExists(hasText("saved to Downloads/clip.wav", substring = true), 5_000)
        assertEquals(listOf(fileAttachmentId(CLIP) to "clip.wav"), io.downloaded)
    }

    @Test
    fun a_node_that_will_not_stream_it_says_so_where_the_button_was() = runComposeUiTest {
        val io = object : PaneIo by SoundNode() {
            override suspend fun streamUrl(paneId: String, id: String): String? = null
        }
        setContent { Screen(paneOf(SPOKEN), io, Decoder()) }
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitUntilAtLeastOneExists(hasText("could not stream", substring = true), 5_000)
    }

    // An hour-long recording is the case streaming exists for, and an hour with no way into the
    // middle of it is a recording nobody can use.
    @Test
    fun a_long_recording_shows_where_it_is_and_can_be_moved_through() = runComposeUiTest {
        val decoder = Decoder(lasts = 45 * 60_000L)
        setContent { Screen(paneOf(SPOKEN), SoundNode(), decoder) }
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitUntilAtLeastOneExists(hasText("0:00 / 45:00"), 5_000)

        onNodeWithContentDescription("Position in clip.wav").performSemanticsAction(SemanticsActions.SetProgress) { it(0.5f) }
        waitUntilAtLeastOneExists(hasText("22:30 / 45:00"), 5_000)
        assertEquals(45 * 30_000L, decoder.latest?.positionMs)
    }

    // The other way to the same player: a recording a transcript carries as an attachment.
    @Test
    fun a_recording_attached_to_a_turn_streams_too() = runComposeUiTest {
        val io = SoundNode()
        val decoder = Decoder()
        setContent { Screen(paneOf(ATTACHED), io, decoder) }
        onNodeWithContentDescription("Play audio, memo.wav").performClick()
        waitUntilAtLeastOneExists(hasContentDescription("Pause memo.wav"), 5_000)

        assertEquals(listOf(VOICE_NOTE), io.streamed)
        assertEquals(TICKET, decoder.latest?.url)
        assertTrue(io.asked.isEmpty(), "the attachment was fetched whole instead of streamed")
    }

    // A ticket dies with the node that minted it. The bar asks for another and plays on from
    // where the recording stopped.
    @Test
    fun a_recording_whose_stream_dropped_plays_on_from_where_it_was_on_a_new_ticket() = runComposeUiTest {
        var mints = 0
        val io = object : PaneIo by SoundNode() {
            override suspend fun streamUrl(paneId: String, id: String): String = "$TICKET?n=${mints++}"
        }
        val decoder = Decoder(lasts = 10 * 60_000L)
        setContent { Screen(paneOf(ATTACHED), io, decoder) }
        onNodeWithContentDescription("Play audio, memo.wav").performClick()
        waitUntilAtLeastOneExists(hasContentDescription("Pause memo.wav"), 5_000)
        val first = requireNotNull(decoder.latest)
        first.seek(90_000L)
        first.dropped()

        waitUntilAtLeastOneExists(hasContentDescription("Try again, memo.wav"), 5_000)
        onNodeWithContentDescription("Try again, memo.wav").performClick()
        waitUntilAtLeastOneExists(hasContentDescription("Pause memo.wav"), 5_000)

        val second = requireNotNull(decoder.latest)
        assertEquals("$TICKET?n=1", second.url)
        assertEquals(90_000L, second.positionMs)
        assertTrue(second.playing, "the new stream was not played on")
    }

    // A device with no decoder for the type must not wear the word "play" at all — the row is the
    // ordinary file row, and the press downloads.
    @Test
    fun a_device_that_cannot_decode_the_type_is_never_offered_a_press_to_play() = runComposeUiTest {
        setContent { Screen(paneOf(SPOKEN), SoundNode(), Decoder(types = emptySet())) }
        waitForIdle()

        assertTrue(
            onAllNodesWithContentDescription("Play clip.wav").fetchSemanticsNodes().isEmpty(),
            "a device with no decoder for the type offered to play it",
        )
        onNodeWithContentDescription("Open clip.wav").assertExists()
    }

    // The whole security argument for a path-shaped id: a device that may type into a terminal can
    // already `cat` the file, and a device that may not is exactly the one that must not reach
    // `~/.ssh/id_rsa`. The route refuses it outright, so the row is absent rather than failing.
    @Test
    fun a_read_only_device_is_offered_no_recording_and_asks_for_nothing() = runComposeUiTest {
        val io = SoundNode(readOnly = true)
        setContent { Screen(paneOf(SPOKEN), io, Decoder()) }
        waitForIdle()

        assertTrue(io.asked.isEmpty(), "a read-only device fetched a file the route would refuse it")
        assertTrue(
            onAllNodesWithContentDescription("clip.wav", substring = true).fetchSemanticsNodes().isEmpty(),
            "a read-only device was offered a recording the node would refuse it",
        )
    }

    @Test
    fun the_prose_that_named_it_is_the_prose_it_always_was() = runComposeUiTest {
        setContent { Screen(paneOf(SPOKEN), SoundNode(), Decoder()) }
        waitForIdle()
        onNodeWithText(CLIP, substring = true).assertExists()
    }

    // The desktop's own answer, asked without opening a line — which this JVM must never do.
    @Test
    fun this_desktop_reads_the_containers_the_jdk_ships_a_decoder_for_and_no_others() {
        assertTrue(deviceVoices.canPlay("audio/wav"))
        assertTrue(deviceVoices.canPlay("AUDIO/WAV; codecs=1"))
        assertFalse(deviceVoices.canPlay("audio/mpeg"))
        assertFalse(deviceVoices.canPlay("audio/flac"))
        assertFalse(deviceVoices.canPlay("text/plain"))
        assertFalse(deviceVoices.canPlay(null))
    }
}
