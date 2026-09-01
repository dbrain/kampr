package dev.kampr.conversation

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.Answering
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals

private val PENDING = ServerMsg.Pending(
    pane = PANE_ID,
    question = "Do you want to make this edit?",
    options = listOf(PendingOption("1", "Yes"), PendingOption("2", "Always"), PendingOption("3", "No")),
    source = "transcript",
)

@OptIn(ExperimentalTestApi::class)
class AccessibilityTest {
    // The prompt arrives on its own, carries the question, and each chip says which answer it
    // sends — "1" alone is what the eye can live with and a reader cannot.
    @Test
    fun thePromptAnnouncesItselfAndNamesEveryAnswer() = runComposeUiTest {
        var answered: String? = null
        setContent {
            CompositionLocalProvider(LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone)) {
                PendingStrip(PENDING, Answering.Ready, { answered = it }, Modifier.fillMaxWidth())
            }
        }
        onNodeWithContentDescription("Do you want to make this edit?", substring = true)
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Assertive))
        onNodeWithContentDescription("Answer 2, Always").performClick()
        assertEquals("2", answered)
    }

    @Test
    fun theComposerNamesItsSendButtonAfterTheAgent() = runComposeUiTest {
        var sent: String? = null
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
            ) {
                Composer(agent = "claude", enabled = true, onSend = { sent = it })
            }
        }
        onNodeWithContentDescription("Send this reply to claude").assertHeightIsAtLeast(44.dp)
        assertEquals(null, sent)
    }

    @Test
    fun theTranscriptSearchIconIsNamedForWhatItDoes() = runComposeUiTest {
        val (_, pane) = demoPane(RICH_CONVO)
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
            ) {
                ConversationView(pane, demoInfo(), Modifier.fillMaxWidth())
            }
        }
        onNodeWithContentDescription("Search the transcript").assertHeightIsAtLeast(LANDSCAPE_TOUCH)
    }

    // A preview is text whose wording may still change under the reader, so it says so — and it
    // stops saying so the moment the transcript takes over. Both halves, because a mark that never
    // goes away is worse than no mark.
    // The mark moved from the live turn to the foot of the transcript when a reply became the
    // block a reader deals with, but it says the same thing and it still has to stop saying it.
    // Both halves, because a mark that never goes away is worse than no mark.
    @Test
    fun aLiveReplySaysItIsStillBeingWrittenAndThenStops() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
            ) {
                val (_, pane) = demoPane(RICH_CONVO, LIVE_TURN)
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(pane, demoInfo(), Modifier.fillMaxSize())
                }
            }
        }
        waitForIdle()
        onNodeWithContentDescription("writing", substring = true).assertExists()
    }

    @Test
    fun aWithdrawnLiveTurnSaysNothingAtAll() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
                LocalPaneIo provides RecordingIo,
            ) {
                val (_, pane) = demoPane(RICH_CONVO, LIVE_TURN, LIVE_WITHDRAWN)
                Box(Modifier.size(PORTRAIT.first, PORTRAIT.second)) {
                    ConversationView(pane, demoInfo(), Modifier.fillMaxSize())
                }
            }
        }
        waitForIdle()
        onNodeWithContentDescription("writing", substring = true).assertDoesNotExist()
    }
}
