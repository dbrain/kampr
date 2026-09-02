package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
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
private fun said(text: String, role: String = "assistant") =
    """{"t":"convo","pane":"$PANE_ID","cursor":"s1","more":false,"turns":[
        {"id":"s1","role":"$role","blocks":[{"b":"md","text":"$text"}]}]}"""

private val SPOKEN = said("rendered it to $CLIP \\u2014 my notes are in $NOTES")

// A decoder this test drives, in place of the device's own. The desktop's is deliberately not used
// here: opening a JDK audio line starts AWT inside the test JVM and deadlocks the Compose harness,
// which is the whole reason [LocalVoices] exists — see the comment on `Audio.jvm.kt`.
private class Decoder(
    private val types: Set<String> = setOf("audio/wav"),
    private val deaf: Boolean = false,
) : Voices {
    var latest: Sounding? = null

    override fun canPlay(mime: String?): Boolean = audioType(mime) in types

    override fun open(bytes: ByteArray, mime: String?): Voice? =
        if (deaf) null else Sounding().also { latest = it }
}

private class Sounding : Voice {
    override var playing = false
        private set

    fun ended() {
        playing = false
    }

    override fun play() {
        playing = true
    }

    override fun pause() {
        playing = false
    }

    override fun release() {
        playing = false
    }
}

private class SoundNode(
    private val type: String? = "audio/wav",
    override val readOnly: Boolean = false,
) : PaneIo {
    val asked = mutableListOf<String>()

    override fun send(msg: ClientMsg) = Unit

    override fun prefs(paneId: String): PanePrefs = PanePrefs()

    // The route answers a media type off its own fixed list, read out of the bytes: a `.wav` with
    // no recorded type comes back as `audio/wav`, and nothing the transcript said decided that.
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes {
        asked += id
        return if (id == fileAttachmentId(CLIP)) {
            AttachmentBytes.Ok(ByteArray(64) { it.toByte() }, type)
        } else {
            AttachmentBytes.Failed("no such attachment")
        }
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

    // One authorised fetch at the id the route reads, and what comes back becomes a player in
    // place rather than a viewer over the pane: a sound has nothing to look at, and the reader
    // goes on reading the transcript under it while it runs.
    @Test
    fun pressing_it_fetches_once_at_the_id_the_route_reads_and_becomes_a_player() = runComposeUiTest {
        val io = SoundNode()
        setContent { Screen(paneOf(SPOKEN), io, Decoder()) }
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitForIdle()

        assertEquals(listOf(fileAttachmentId(CLIP)), io.asked)
        onNodeWithContentDescription("Save clip.wav").assertExists()
        assertTrue(
            onAllNodesWithText("saved to", substring = true).fetchSemanticsNodes().isEmpty(),
            "the bytes went straight to the downloads folder instead of to a player",
        )
        assertTrue(
            onAllNodesWithContentDescription("Read clip.wav").fetchSemanticsNodes().isEmpty(),
            "a recording was offered as a document to read",
        )
    }

    // The press the operator asked for, end to end through a real composed transcript: the button
    // reaches the decoder, and the glyph on it says which way the next press goes.
    @Test
    fun the_second_press_starts_the_sound_and_turns_the_button_into_a_pause() = runComposeUiTest {
        val decoder = Decoder()
        setContent { Screen(paneOf(SPOKEN), SoundNode(), decoder) }
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitForIdle()
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitForIdle()

        onNodeWithContentDescription("Pause clip.wav").assertExists()
        assertTrue(decoder.latest?.playing == true, "the button was pressed and nothing was played")

        onNodeWithContentDescription("Pause clip.wav").performClick()
        waitForIdle()
        assertFalse(decoder.latest?.playing == true, "pausing left the sound running")
        onNodeWithContentDescription("Play clip.wav").assertExists()
    }

    // A sound that runs to its end is not paused by anybody, and the button has to come back on
    // its own or the reader is left looking at a pause that would do nothing.
    @Test
    fun a_sound_that_reaches_its_end_puts_its_own_button_back() = runComposeUiTest {
        val decoder = Decoder()
        setContent { Screen(paneOf(SPOKEN), SoundNode(), decoder) }
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitForIdle()
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitForIdle()
        onNodeWithContentDescription("Pause clip.wav").assertExists()

        decoder.latest?.ended()
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription("Play clip.wav").fetchSemanticsNodes().isNotEmpty()
        }
    }

    // A type this device claims and bytes it then cannot read. The reader is told, and is still
    // given the file — which is the floor the operator asked for.
    @Test
    fun bytes_a_decoder_will_not_take_say_so_and_leave_the_file_on_offer() = runComposeUiTest {
        setContent { Screen(paneOf(SPOKEN), SoundNode(), Decoder(deaf = true)) }
        onNodeWithContentDescription("Play clip.wav").performClick()
        waitForIdle()

        onNodeWithText("would not play it", substring = true).assertExists()
        onNodeWithContentDescription("Save clip.wav").assertExists()
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
