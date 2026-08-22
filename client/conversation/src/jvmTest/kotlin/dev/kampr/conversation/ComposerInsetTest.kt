package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.BottomEdgeHeldBelow
import dev.kampr.shared.ui.BottomNav
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.ui.Tab
import dev.kampr.shared.ui.keyboardInset
import kotlin.test.Test
import kotlin.test.assertTrue

private val BARS = SafeArea(top = 32.dp, bottom = 24.dp)
private val KEYBOARD = BARS.copy(ime = 300.dp)
private const val REPLY = "Reply to claude"
private const val SEND = "Send this reply"

// Rotated: the gesture handle thins and a three-button bar leaves the bottom for one side.
private val ROTATED = listOf(
    SafeArea(top = 24.dp, bottom = 24.dp),
    SafeArea(top = 24.dp, bottom = 0.dp, left = 48.dp),
    SafeArea(top = 24.dp, bottom = 0.dp, right = 48.dp),
)

// What the phone actually stacks: the app's root, the pane inside it, the bottom navigation under
// that — which is what holds the gesture handle off, and says so. The reply box is the last thing
// in the conversation and the first thing a keyboard covers.
//
// `nav = false` is the rotated pane, which wears no tab bar at all: there the reply box is what
// ends at the window and owes the handle itself.
@OptIn(ExperimentalTestApi::class)
@Composable
private fun Phone(safe: SafeArea, nav: Boolean = true, content: @Composable () -> Unit) {
    CompositionLocalProvider(
        LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone),
        LocalPaneIo provides RecordingIo,
        LocalSafeArea provides safe,
    ) {
        Box(Modifier.fillMaxSize().keyboardInset()) {
            Column(Modifier.fillMaxSize()) {
                Box(Modifier.weight(1f)) { BottomEdgeHeldBelow(nav, content) }
                if (nav) BottomNav(Tab.Herd, {})
            }
        }
    }
}

@OptIn(ExperimentalTestApi::class)
class ComposerInsetTest {
    // The report, verbatim: "typing in conversation window doesn't show you the text entry box
    // (its under keyboard)". The window is not resized when the keyboard opens, so the reply box
    // stayed exactly where it was — 300 dp below the top of the keys. Both halves matter: a fix
    // that pushed it off the top instead would satisfy the first assertion and nothing else.
    @Test
    fun theReplyBoxStaysAboveTheKeyboardAndOnScreen() = runComposeUiTest {
        val (_, pane) = demoPane(RICH_CONVO)
        setContent { Phone(KEYBOARD) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) } }
        val screen = onRoot().getUnclippedBoundsInRoot()
        val reply = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
        assertTrue(
            reply.bottom <= screen.bottom - KEYBOARD.ime,
            "the reply box reaches ${reply.bottom}, and the keys start at ${screen.bottom - KEYBOARD.ime}",
        )
        assertTrue(reply.top >= screen.top, "the reply box starts at ${reply.top}, above the window")
        assertTrue(reply.bottom > reply.top, "the reply box measured ${reply.top}..${reply.bottom}")
    }

    // With the keyboard closed nothing moves, or every conversation loses a strip to a keyboard
    // that is not there.
    @Test
    fun aClosedKeyboardLeavesTheConversationWhereItWas() = runComposeUiTest {
        val (_, pane) = demoPane(RICH_CONVO)
        setContent { Phone(BARS) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) } }
        val screen = onRoot().getUnclippedBoundsInRoot()
        val nav = onNodeWithContentDescription("Herd tab").getUnclippedBoundsInRoot()
        assertTrue(
            nav.bottom <= screen.bottom && nav.bottom > screen.bottom - 100.dp,
            "the bottom navigation is at ${nav.bottom} of ${screen.bottom}",
        )
    }

    // The last turn is what you are replying to, and a lazy list anchors on its first visible item
    // — so when the keyboard halves the viewport the turn slides out from under you. The reply box
    // has to ride up with the keys, and the end of the transcript has to still be under it.
    @Test
    fun theTranscriptFollowsTheKeyboardUp() = runComposeUiTest {
        val (_, pane) = demoPane(RICH_CONVO)
        var safe by mutableStateOf(BARS)
        setContent { Phone(safe) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) } }
        waitForIdle()
        val closed = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
        assertTrue(lastTurnIsComposed(), "the transcript did not start at its end")
        safe = KEYBOARD
        waitForIdle()
        val open = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
        assertTrue(
            open.bottom <= closed.bottom - KEYBOARD.ime + BARS.ime,
            "the reply box was at ${closed.bottom} and the keyboard moved it only to ${open.bottom}",
        )
        assertTrue(lastTurnIsComposed(), "the last turn scrolled out when the keyboard opened")
    }

    // The third container, and the one nothing was holding: rotate the phone onto a pane and the
    // tab bar goes, so the reply box is the last thing in the window and the gesture handle lands
    // on it. Rotated with three-button navigation the bar takes a side instead, which is where the
    // send button is.
    @Test
    fun aRotatedPaneHasNothingUnderTheReplyBoxButTheReplyBox() {
        for (bars in ROTATED) {
            runComposeUiTest {
                val (_, pane) = demoPane(RICH_CONVO)
                setContent { Phone(bars, nav = false) { ConversationView(pane, demoInfo(), Modifier.fillMaxSize()) } }
                val screen = onRoot().getUnclippedBoundsInRoot()
                val reply = onNodeWithContentDescription(REPLY).getUnclippedBoundsInRoot()
                val send = onNodeWithContentDescription(SEND, substring = true).getUnclippedBoundsInRoot()
                assertTrue(
                    reply.bottom <= screen.bottom - bars.bottom,
                    "$bars: the reply box reaches ${reply.bottom} of ${screen.bottom}, inside the " +
                        "${bars.bottom} the system draws its gesture handle in",
                )
                assertTrue(
                    reply.left >= bars.left && send.right <= screen.right - bars.right,
                    "$bars: the composer spans ${reply.left}..${send.right} of ${screen.right}, " +
                        "inside the bar that the rotation moved to the side",
                )
            }
        }
    }

    // RICH_CONVO ends on a running `Edit` card, and a lazy list only composes what is in view.
    private fun ComposeUiTest.lastTurnIsComposed(): Boolean =
        onAllNodesWithContentDescription("Edit", substring = true).fetchSemanticsNodes().isNotEmpty()
}
