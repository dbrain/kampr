package dev.kampr.conversation

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.KeyInjectionScope
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.requestFocus
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.test.withKeyDown
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
private fun ComposeUiTest.composer(drafts: MutableList<String> = mutableListOf()) {
    setContent {
        CompositionLocalProvider(
            LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
            LocalSafeArea provides SafeArea(top = 0.dp, bottom = 0.dp),
            LocalHardKeyboard provides true,
        ) {
            Composer("claude", enabled = true, onSend = {}, onDraft = { drafts += it })
        }
    }
    onNodeWithContentDescription(REPLY).requestFocus()
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.editable(): String =
    onNodeWithContentDescription(REPLY).fetchSemanticsNode().config[SemanticsProperties.EditableText].text

private fun KeyInjectionScope.ctrl(key: Key) = withKeyDown(Key.CtrlLeft) { pressKey(key) }

private fun KeyInjectionScope.left(times: Int) = repeat(times) { pressKey(Key.DirectionLeft) }

// The caret is not readable off the semantics of a `BasicText`, so every motion is proved by
// typing a mark at wherever it landed. `then` is that mark; the cases with none are the kills,
// which say where the caret went by what is left.
private class Case(
    val what: String,
    val typed: String,
    val press: KeyInjectionScope.() -> Unit,
    val then: String = "",
    val expect: String,
)

private val CASES = listOf(
    Case("ctrl+A", "one two", { ctrl(Key.A) }, then = "|", expect = "|one two"),
    Case("ctrl+E", "one two", { ctrl(Key.A); ctrl(Key.E) }, then = "|", expect = "one two|"),
    Case("ctrl+U on the whole line", "one two", { ctrl(Key.U) }, expect = ""),
    Case("ctrl+U behind the caret", "one two", { left(3); ctrl(Key.U) }, expect = "two"),
    Case("ctrl+K", "one two", { left(3); ctrl(Key.K) }, expect = "one "),
    Case("ctrl+W", "one two", { ctrl(Key.W) }, expect = "one "),
    Case("ctrl+W twice", "one two", { ctrl(Key.W); ctrl(Key.W) }, expect = ""),
)

// The operator's ask, in their words: "on bash i can ctrl+A ctrl+E ctrl+U etc. to move to
// start/end of line — could we support that on the conversation text entry".
@OptIn(ExperimentalTestApi::class)
class ComposerLineKeysTest {
    @Test
    fun theReplyBoxTakesTheShellsLineEditingKeys() {
        for (case in CASES) {
            runComposeUiTest {
                composer()
                onNodeWithContentDescription(REPLY).performTextInput(case.typed)
                onNodeWithContentDescription(REPLY).performKeyInput(case.press)
                if (case.then.isNotEmpty()) {
                    onNodeWithContentDescription(REPLY).performTextInput(case.then)
                }
                assertEquals(case.expect, editable(), "${case.what} left the box wrong")
            }
        }
    }

    @Test
    fun ctrlAOnASecondLineStopsAtThatLinesStart() = runComposeUiTest {
        composer()
        onNodeWithContentDescription(REPLY).performTextInput("first")
        onNodeWithContentDescription(REPLY).performKeyInput {
            withKeyDown(Key.ShiftLeft) { pressKey(Key.Enter) }
        }
        onNodeWithContentDescription(REPLY).performTextInput("second")
        onNodeWithContentDescription(REPLY).performKeyInput { ctrl(Key.A) }
        onNodeWithContentDescription(REPLY).performTextInput("|")
        assertEquals("first\n|second", editable(), "ctrl+A left the line it was editing")
    }

    // The whole reason the other three edits in `Composer` report by hand: an input transformation
    // does not see `state.edit`, and a draft nobody reported is a reply lost to a glance at the
    // terminal view, which is what takes this composable out of the composition.
    @Test
    fun aKilledLineIsReportedAsTheDraft() = runComposeUiTest {
        val drafts = mutableListOf<String>()
        composer(drafts)
        onNodeWithContentDescription(REPLY).performTextInput("one two")
        onNodeWithContentDescription(REPLY).performKeyInput { ctrl(Key.W) }
        assertEquals("one ", drafts.lastOrNull(), "the draft still reads ${drafts.lastOrNull()}")
    }

    // Undo is the platform's own — ctrl+Z, ctrl+Y and ctrl+shift+Z are in Compose's common key
    // mapping — and this is here to say so out loud: the readline keys above take ctrl+A out of
    // that mapping, and nothing else may follow it out.
    @Test
    fun undoAndRedoAreStillTheKeyboardsOwn() = runComposeUiTest {
        composer()
        onNodeWithContentDescription(REPLY).performTextInput("one two")
        onNodeWithContentDescription(REPLY).performKeyInput { ctrl(Key.W) }
        assertEquals("one ", editable())
        onNodeWithContentDescription(REPLY).performKeyInput { ctrl(Key.Z) }
        assertEquals("one two", editable(), "ctrl+Z did not put the killed word back")
        onNodeWithContentDescription(REPLY).performKeyInput { ctrl(Key.Y) }
        assertEquals("one ", editable(), "ctrl+Y did not redo the kill")
    }
}
