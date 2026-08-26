package dev.kampr.conversation

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.SafeArea
import kotlin.test.Test
import kotlin.test.assertEquals

private const val REPLY = "Reply to claude"

// The terminal's input field emptied itself after every committed character, which is what handed
// Gboard a restartInput per keystroke and sent it back to its letters page mid-IP-address. The
// reply box was reported as maybe doing the same; it hands back exactly what the IME put in it,
// and this is what says so.
@OptIn(ExperimentalTestApi::class)
class ComposerTest {
    @Test
    fun the_reply_box_keeps_every_digit_the_keyboard_commits() = runComposeUiTest {
        val sent = mutableListOf<String>()
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalSafeArea provides SafeArea(top = 0.dp, bottom = 0.dp),
            ) {
                Composer("claude", enabled = true, onSend = { sent += it })
            }
        }
        for (ch in "192.168.1.1") onNodeWithContentDescription(REPLY).performTextInput(ch.toString())
        assertEquals("192.168.1.1", editable())
        onNodeWithContentDescription("Send this reply to claude").performClick()
        assertEquals(listOf("192.168.1.1"), sent)
        assertEquals("", editable(), "a sent reply has to leave the box empty")
    }

    @OptIn(ExperimentalTestApi::class)
    private fun androidx.compose.ui.test.ComposeUiTest.editable(): String =
        onNodeWithContentDescription(REPLY).fetchSemanticsNode()
            .config[SemanticsProperties.EditableText].text
}
