package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.click
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.input.Esc
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.PaneKeyRow
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

private class Taps : PaneIo {
    val text = mutableListOf<String>()
    override fun send(msg: ClientMsg) {
        if (msg is ClientMsg.InputText) text += msg.text
    }

    override fun prefs(paneId: String): PanePrefs = PanePrefs()

    fun count(of: String) = text.count { it == of }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.keyRow(io: Taps) {
    setContent {
        CompositionLocalProvider(
            LocalTokens provides KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
                .let { KamprTokens(SoftTheme, it, typography(it, SoftTheme.label, TypeScale.Phone)) },
            LocalSafeArea provides SafeArea(top = 32.dp, bottom = 46.dp),
        ) {
            val session = PaneSession(PANE)
            Box(Modifier.fillMaxSize()) {
                PaneKeyRow(session, InputSink(PANE, io, session.latches), compact = false, enabled = true)
            }
        }
    }
    waitForIdle()
}

// The operator, on 0.1.57: *"the arrow keys on the virtual keyboard thing when looking at terminal
// on mobile - if i hold one can we have it repeat the keypress?"*
//
// A cap answered a press and a long press with one key each, so walking a shell's history or a
// line of text was one tap per row — which is what a physical keyboard's autorepeat exists to
// stop. Only the arrows repeat: every other cap on the row already spends its long press on
// something (an alternate, a latch, a lock), and taking that away to add a repeat would be a worse
// trade than the one being fixed.
@OptIn(ExperimentalTestApi::class)
class HoldingAKeyCapTest {
    @Test
    fun holding_an_arrow_repeats_it_until_the_finger_leaves() = runComposeUiTest {
        val io = Taps()
        keyRow(io)
        val arrow = onNodeWithContentDescription("Down arrow", substring = true)

        arrow.performTouchInput { down(center) }
        waitUntil(timeoutMillis = 5_000) { io.count(Esc.DOWN) >= 4 }
        arrow.performTouchInput { up() }

        val held = io.count(Esc.DOWN)
        mainClock.advanceTimeBy(2_000)
        waitForIdle()
        assertEquals(held, io.count(Esc.DOWN), "the key went on repeating after the finger left")
        assertTrue(
            io.text.all { it == Esc.DOWN },
            "holding one arrow sent something else as well: ${io.text.distinct()}",
        )
    }

    // A tap is still a keystroke. The repeat waits out a delay first, so the ordinary press has to
    // be exactly one key and not two.
    @Test
    fun a_tap_on_an_arrow_is_one_key() = runComposeUiTest {
        val io = Taps()
        keyRow(io)
        onNodeWithContentDescription("Up arrow", substring = true).performTouchInput { click(center) }
        waitForIdle()
        mainClock.advanceTimeBy(2_000)
        waitForIdle()
        assertEquals(1, io.count(Esc.UP), "a tap sent ${io.text}")
    }

    // And the caps whose long press is already spoken for keep it: `home` holds to `ins`, and it
    // must not have become a run of `home`s.
    @Test
    fun a_cap_whose_long_press_is_spoken_for_keeps_it() = runComposeUiTest {
        val io = Taps()
        keyRow(io)
        onNodeWithContentDescription("Home", substring = true).performTouchInput { longClick(center) }
        waitForIdle()
        mainClock.advanceTimeBy(2_000)
        waitForIdle()
        assertEquals(listOf(Esc.INSERT), io.text)
    }
}
