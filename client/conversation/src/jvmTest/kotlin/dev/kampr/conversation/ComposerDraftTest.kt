package dev.kampr.conversation

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
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

// A half-written reply used to live in the composer's own `remember`, so it died the moment the
// composer left the composition — which is what switching to the terminal view does. The draft
// belongs to the pane, not to the box that happens to be showing it.
@OptIn(ExperimentalTestApi::class)
class ComposerDraftTest {
    @Test
    fun a_half_written_reply_survives_a_look_at_the_terminal() = runComposeUiTest {
        var draft by mutableStateOf("")
        var showing by mutableStateOf(true)
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalSafeArea provides SafeArea(top = 0.dp, bottom = 0.dp),
            ) {
                if (showing) {
                    Composer("claude", enabled = true, onSend = {}, draft = draft, onDraft = { draft = it })
                }
            }
        }
        onNodeWithContentDescription(REPLY).performTextInput("half a thought")
        assertEquals("half a thought", draft, "the pane holds it, not the box")

        showing = false
        waitForIdle()
        showing = true
        waitForIdle()

        assertEquals("half a thought", editable(), "and it is still there on the way back")
    }

    @Test
    fun a_sent_reply_leaves_nothing_behind_for_the_next_visit() = runComposeUiTest {
        var draft by mutableStateOf("")
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalSafeArea provides SafeArea(top = 0.dp, bottom = 0.dp),
            ) {
                Composer("claude", enabled = true, onSend = {}, draft = draft, onDraft = { draft = it })
            }
        }
        onNodeWithContentDescription(REPLY).performTextInput("say this")
        onNodeWithContentDescription("Send this reply to claude").performClick()
        assertEquals("", draft, "a sent reply is not a draft to come back to")
        assertEquals("", editable())
    }

    @OptIn(ExperimentalTestApi::class)
    private fun androidx.compose.ui.test.ComposeUiTest.editable(): String =
        onNodeWithContentDescription(REPLY).fetchSemanticsNode()
            .config[SemanticsProperties.EditableText].text
}
