package dev.kampr.terminal

import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.withKeyDown
import androidx.compose.ui.text.AnnotatedString
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.guard.ConfirmState
import dev.kampr.terminal.guard.SubmitGuard
import dev.kampr.terminal.input.Esc
import dev.kampr.terminal.input.FieldTextInput
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.KeyCap
import dev.kampr.terminal.input.Latch
import dev.kampr.terminal.input.active
import dev.kampr.terminal.input.Latches
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private const val TAG = "pane-input"
private const val PANE = "01JKAMPRNODE0000000000000/w1:p1"

private class Typed : PaneIo {
    val sent = mutableListOf<String>()

    override fun send(msg: ClientMsg) {
        if (msg is ClientMsg.InputText) sent += msg.text
    }

    override fun prefs(paneId: String) = PanePrefs()

    val all: String get() = sent.joinToString("")
    val backspaces: Int get() = all.count { it == '\u007F' }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.field(
    latches: Latches = Latches(),
    guard: (PaneIo) -> SubmitGuard? = { null },
): Typed = rig(latches, guard).first

// The shape every pane on screen actually has: `TerminalView` and `TerminalSurfaces` both build
// their sink with a guard, and the guard is the only thing that ever cuts a payload up. A rig
// without one cannot see what the wire is handed.
private fun guarded(io: PaneIo): SubmitGuard {
    val pane = PaneState(PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = PANE,
            cols = 80,
            rows = 1,
            rowsData = listOf(RowDiff(0, listOf(Run(0, "> ")))),
            cursor = Cursor(2, 0, true),
            links = emptyList(),
        ),
    )
    return SubmitGuard(pane, io, ConfirmState())
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.rig(
    latches: Latches = Latches(),
    guard: (PaneIo) -> SubmitGuard? = { null },
): Pair<Typed, InputSink> {
    val io = Typed()
    val sink = InputSink(PANE, io, latches, guard(io))
    setContent {
        val session = remember { PaneSession(PANE).also { it.openKeyboard() } }
        FieldTextInput(session, sink, enabled = true, onChord = {}, modifier = Modifier.testTag(TAG))
    }
    waitForIdle()
    return io to sink
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.buffer(): String =
    onNodeWithTag(TAG).fetchSemanticsNode().config[SemanticsProperties.EditableText].text

@OptIn(ExperimentalTestApi::class)
class FieldTextInputTest {
    // The defect: the field was emptied after every committed character, which hands the IME a
    // different editor state than the one it just produced. On Android that is a restartInput on
    // every keystroke, and a restarted Gboard drops back to its letters page — so a typed IP
    // address reverts to letters after the first digit.
    @Test
    fun a_committed_digit_leaves_the_editor_holding_what_the_ime_typed() = runComposeUiTest {
        val io = field()
        val before = buffer()
        onNodeWithTag(TAG).performTextInput("1")
        assertEquals("1", io.all)
        assertEquals(before + "1", buffer(), "the field was reset under the IME")
    }

    @Test
    fun an_ip_address_arrives_one_character_at_a_time_and_in_order() = runComposeUiTest {
        val io = field()
        val before = buffer()
        for (ch in "192.168.1.1") onNodeWithTag(TAG).performTextInput(ch.toString())
        assertEquals("192.168.1.1", io.all)
        assertEquals(0, io.backspaces, "ordinary typing must never send a backspace")
        assertEquals(before + "192.168.1.1", buffer())
    }

    // Two commits inside one frame, which is what typing faster than a recomposition looks like.
    // A field whose baseline is a value handed back through recomposition diffs the second commit
    // against a baseline that has not landed yet and types the first character twice.
    @Test
    fun two_commits_inside_one_frame_are_not_typed_twice() = runComposeUiTest {
        val io = field()
        val insert = onNodeWithTag(TAG).fetchSemanticsNode()
            .config[SemanticsActions.InsertTextAtCursor].action!!
        runOnUiThread {
            insert(AnnotatedString("1"))
            insert(AnnotatedString("9"))
        }
        waitForIdle()
        assertEquals("19", io.all)
    }

    @Test
    fun a_run_of_commits_inside_one_frame_arrives_once_each() = runComposeUiTest {
        val io = field()
        val insert = onNodeWithTag(TAG).fetchSemanticsNode()
            .config[SemanticsActions.InsertTextAtCursor].action!!
        runOnUiThread { for (ch in "192.168.1.1") insert(AnnotatedString(ch.toString())) }
        waitForIdle()
        assertEquals("192.168.1.1", io.all)
    }

    // A soft keyboard sends backspace either as a key event or as a deletion in the buffer,
    // depending on whether it can see anything in front of the cursor. Both have to end in
    // exactly one backspace on the wire — the pad makes the second path live, and a double
    // delete would be the regression it introduces.
    @Test
    fun a_backspace_key_event_reaches_the_pane_exactly_once() = runComposeUiTest {
        val io = field()
        val before = buffer()
        onNodeWithTag(TAG).performKeyInput { pressKey(Key.Backspace) }
        waitForIdle()
        assertEquals(Esc.BACKSPACE, io.all)
        assertEquals(
            before.length - 1,
            buffer().length,
            "one keystroke is one character: the buffer either kept a character the pane deleted " +
                "or deleted two for one press",
        )
    }

    // The report: *"backspacing deletes from terminal but keyboard autocomplete keeps the mistyped
    // text"*. This buffer is where the IME reads the line back out of — its composing region, its
    // suggestion strip and its correction all come from it — and it only ever grew: the key path
    // consumes the backspace so the pane loses a character and the editor does not. Accepting the
    // suggestion then types a correction to a word that has not been on the pane for three
    // keystrokes. So the buffer follows the line: a character off for a backspace, and back to
    // bare padding for anything that ends the line or moves the caret off it.
    @Test
    fun a_backspace_takes_the_character_off_the_editor_the_keyboard_is_reading() = runComposeUiTest {
        val io = field()
        onNodeWithTag(TAG).performTextInput("helo")
        val typed = buffer()
        io.sent.clear()
        onNodeWithTag(TAG).performKeyInput { pressKey(Key.Backspace) }
        waitForIdle()
        assertEquals(Esc.BACKSPACE, io.all, "and still exactly one backspace on the wire")
        assertEquals(
            typed.dropLast(1),
            buffer(),
            "the keyboard is still autocompleting a word the pane no longer holds",
        )
    }

    @Test
    fun the_enter_that_ran_the_command_leaves_no_line_behind_in_the_editor() = runComposeUiTest {
        field()
        onNodeWithTag(TAG).performTextInput("cargo test")
        onNodeWithTag(TAG).performKeyInput { pressKey(Key.Enter) }
        waitForIdle()
        assertFalse("cargo test" in buffer(), "the command the pane has already run is still the editor's line")
        assertTrue(buffer().isNotEmpty(), "and an empty field has nothing for the next backspace to delete")
    }

    // The key row does not pass through this field at all — `InputSink.press` writes straight to
    // the pane — so a cap that moves the caret leaves the editor holding a line that is no longer
    // under it.
    @Test
    fun a_cap_pressed_on_the_key_row_is_a_line_the_editor_no_longer_holds() = runComposeUiTest {
        val (_, sink) = rig()
        onNodeWithTag(TAG).performTextInput("sttaus")
        runOnUiThread { sink.press(KeyCap("\u2190", send = Esc.LEFT, csi = true)) }
        waitForIdle()
        assertFalse("sttaus" in buffer(), "the caret moved off the word and the keyboard was not told")
    }

    @Test
    fun a_deletion_that_reaches_the_buffer_sends_exactly_one_backspace() = runComposeUiTest {
        val io = field()
        onNodeWithTag(TAG).performTextInput("x")
        io.sent.clear()
        onNodeWithTag(TAG).performTextReplacement(buffer().dropLast(1))
        assertEquals(Esc.BACKSPACE, io.all)
    }

    // Nothing the field keeps to stay non-empty may ever reach the pane.
    @Test
    fun the_padding_that_keeps_the_field_occupied_is_never_typed_at_the_pane() = runComposeUiTest {
        val io = field()
        assertTrue(buffer().isNotEmpty(), "an empty field has nothing for a backspace to delete")
        assertEquals("", io.all, "seeding the field must not type anything")
        onNodeWithTag(TAG).performTextInput("a")
        assertEquals("a", io.all)
    }

    @Test
    fun a_field_worn_down_by_deletions_is_padded_again_before_it_can_empty() = runComposeUiTest {
        val io = field()
        onNodeWithTag(TAG).performTextReplacement("ab")
        io.sent.clear()
        waitForIdle()
        assertTrue(buffer().length > 2, "the field was left one backspace away from empty")
        assertEquals("", io.all, "re-padding the field must not type anything")
    }

    @Test
    fun a_multi_character_commit_arrives_whole() = runComposeUiTest {
        val io = field()
        onNodeWithTag(TAG).performTextInput("kampr doctor")
        assertEquals(listOf("kampr doctor"), io.sent)
    }

    @Test
    fun a_control_chord_still_goes_out_as_its_control_byte() = runComposeUiTest {
        val io = field()
        onNodeWithTag(TAG).performKeyInput { withKeyDown(Key.CtrlLeft) { pressKey(Key.C) } }
        waitForIdle()
        assertEquals("\u0003", io.all)
    }

    @Test
    fun the_arrow_keys_still_go_out_as_escape_sequences() = runComposeUiTest {
        val io = field()
        onNodeWithTag(TAG).performKeyInput { pressKey(Key.DirectionUp) }
        waitForIdle()
        assertEquals(Esc.UP, io.all)
    }

    // The padding is restored from the app rather than the keyboard, and it is the one write this
    // file makes that the IME did not ask for. A commit landing in the same frame as that write
    // has to be diffed against the buffer the IME is editing — against a copy the app keeps, it
    // would be measured from padding that the editor has not been given yet.
    @Test
    fun a_commit_in_the_same_frame_as_a_re_pad_is_diffed_against_the_editor() = runComposeUiTest {
        val io = field()
        val insert = onNodeWithTag(TAG).fetchSemanticsNode()
            .config[SemanticsActions.InsertTextAtCursor].action!!
        val replace = onNodeWithTag(TAG).fetchSemanticsNode()
            .config[SemanticsActions.SetText].action!!
        mainClock.autoAdvance = false
        runOnUiThread { replace(AnnotatedString("ab")) }
        mainClock.advanceTimeByFrame()
        io.sent.clear()
        runOnUiThread { insert(AnnotatedString("x")) }
        mainClock.autoAdvance = true
        waitForIdle()
        assertEquals("x", io.all)
    }

    // The report: alt+enter opens a new line in an agent's prompt box on a desktop keyboard, and
    // on a phone the same chord sent the message instead. A soft keyboard delivers its action key
    // as a real key event carrying no modifier state of its own, and this path read only the
    // hardware modifiers — so an Alt armed on the key row was dropped on the one chord that needed
    // it, while the row's own keys carried it perfectly well through `InputSink.press`.
    @Test
    fun an_armed_alt_rides_the_enter_a_soft_keyboard_sends() = runComposeUiTest {
        val latches = Latches()
        val io = field(latches)

        onNodeWithTag(TAG).performKeyInput { pressKey(Key.Enter) }
        waitForIdle()
        assertEquals(Esc.ENTER, io.sent.last(), "an unmodified enter is still a bare return")

        latches.tap(Latch.Alt)
        onNodeWithTag(TAG).performKeyInput { pressKey(Key.Enter) }
        waitForIdle()
        assertEquals(
            Esc.ESCAPE + Esc.ENTER,
            io.sent.last(),
            "an armed alt has to reach the keystroke the operator armed it for",
        )
        assertFalse(latches.alt.active(), "and it is spent on that keystroke, not left standing")
    }

    // The same chord off a hardware keyboard, through the sink shape a real pane has. `ESC` and
    // `CR` are one keystroke only while they arrive in one `read`, and the guard used to cut every
    // payload carrying a submit in two whether it kept anything back or not — so the pane got an
    // Escape and then an Enter, a `pane.send_text` round trip apart, and the agent submitted the
    // message rather than opening a line in it.
    @Test
    fun alt_enter_reaches_the_pane_as_one_keystroke_rather_than_an_escape_and_an_enter() =
        runComposeUiTest {
            val io = field(guard = ::guarded)

            onNodeWithTag(TAG).performKeyInput { withKeyDown(Key.AltLeft) { pressKey(Key.Enter) } }
            waitForIdle()
            assertEquals(
                listOf(Esc.ESCAPE + Esc.ENTER),
                io.sent,
                "alt+enter was cut into an Escape and an Enter on its way to the pane",
            )
        }
}
