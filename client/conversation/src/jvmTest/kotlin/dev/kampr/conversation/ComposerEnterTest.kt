package dev.kampr.conversation

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.requestFocus
import androidx.compose.ui.test.withKeyDown
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.platform.LocalHardKeyboard
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.SafeArea
import kotlin.test.Test
import kotlin.test.assertEquals

private const val REPLY = "Reply to claude"

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.compose(keyboard: Boolean, sent: MutableList<String>) {
    setContent {
        CompositionLocalProvider(
            LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
            LocalSafeArea provides SafeArea(top = 0.dp, bottom = 0.dp),
            LocalHardKeyboard provides keyboard,
        ) {
            Composer("claude", enabled = true, onSend = { sent += it })
        }
    }
    onNodeWithContentDescription(REPLY).requestFocus()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.editable(): String =
    onNodeWithContentDescription(REPLY).fetchSemanticsNode().config[SemanticsProperties.EditableText].text

// What a keyboard makes people expect, and what every agent CLI on the other end of this pane
// already does: return sends, and a modifier with it is how you get a second line. On a phone it
// must not — there `return` is the only newline the soft keyboard offers, and taking it would
// leave no way to write a paragraph at all.
@OptIn(ExperimentalTestApi::class)
class ComposerEnterTest {
    @Test
    fun returnSendsTheReplyWhenThereIsAKeyboardToPressIt() = runComposeUiTest {
        val sent = mutableListOf<String>()
        compose(keyboard = true, sent = sent)
        onNodeWithContentDescription(REPLY).performTextInput("run the tests")
        onNodeWithContentDescription(REPLY).performKeyInput { pressKey(Key.Enter) }
        assertEquals(listOf("run the tests"), sent)
        assertEquals("", editable(), "a sent reply has to leave the box empty")
    }

    @Test
    fun aModifierWithReturnWritesTheSecondLineInstead() = runComposeUiTest {
        for (modifier in listOf(Key.ShiftLeft, Key.AltLeft)) {
            val sent = mutableListOf<String>()
            runComposeUiTest {
                compose(keyboard = true, sent = sent)
                onNodeWithContentDescription(REPLY).performTextInput("first")
                onNodeWithContentDescription(REPLY).performKeyInput {
                    withKeyDown(modifier) { pressKey(Key.Enter) }
                }
                onNodeWithContentDescription(REPLY).performTextInput("second")
                assertEquals(emptyList(), sent, "$modifier and return sent the reply")
                assertEquals("first\nsecond", editable(), "$modifier and return wrote no second line")
            }
        }
    }

    // A phone's return key is its only newline. Taking it would mean a reply of one line for ever,
    // which is a worse trade than a send button that is already on the screen beside the box.
    @Test
    fun returnOnAPhoneWritesTheSecondLineAndSendsNothing() = runComposeUiTest {
        val sent = mutableListOf<String>()
        compose(keyboard = false, sent = sent)
        onNodeWithContentDescription(REPLY).performTextInput("first")
        onNodeWithContentDescription(REPLY).performKeyInput { pressKey(Key.Enter) }
        onNodeWithContentDescription(REPLY).performTextInput("second")
        assertEquals(emptyList(), sent, "return on a phone sent the reply")
        assertEquals("first\nsecond", editable())
    }

    // Nothing to send, and nothing worth putting in the box either: a blank line typed into an
    // empty reply is not a reply, and the send button beside it is already dark.
    @Test
    fun returnOnAnEmptyBoxDoesNothingAtAll() = runComposeUiTest {
        val sent = mutableListOf<String>()
        compose(keyboard = true, sent = sent)
        onNodeWithContentDescription(REPLY).performKeyInput { pressKey(Key.Enter) }
        assertEquals(emptyList(), sent)
        assertEquals("", editable(), "return on an empty box left something in it")
        onNodeWithContentDescription(REPLY).assertIsDisplayed()
    }
}
