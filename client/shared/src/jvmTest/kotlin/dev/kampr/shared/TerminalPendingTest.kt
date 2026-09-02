package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

private val INFO = PaneInfo(
    id = PANE,
    nodeId = "01JNODE",
    workspace = "kampr",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = "blocked",
    cols = 94,
    rows = 40,
)

private object NoSurfaces : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)
}

private fun blockedPane(): PaneState {
    val store = KamprStore()
    store.accept(
        ServerMsg.Pending(
            pane = PANE,
            question = "Do you want to make this edit?",
            options = listOf(PendingOption("1", "Yes"), PendingOption("2", "Always"), PendingOption("3", "No")),
            source = "screen",
        ),
    )
    return store.pane(PANE)
}

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

// The operator, on 0.1.51: *"terminal pane probably doesn't need buttons at all tbh, if you're on
// the terminal you'll answer on the terminal, its more of a notification/conversation pane thing"*.
//
// **The dialog is already on the screen here.** A terminal view is showing the grid the harness
// drew the question on, and the key row under it types into that same pane — so a chip was a
// second way to press a key the reader could already see and already reach, and it took a band of
// a phone screen off the very grid that says what is being answered. The conversation is the
// surface that needs the card, because there the dialog is not drawn at all.
//
// What a terminal cannot say for itself is that the pane is *blocked* — herdr's status is not on
// the grid — so the badge stays. That is the whole of what this screen owes the operator now.
@OptIn(ExperimentalTestApi::class)
class TerminalPendingTest {
    @Test
    fun theTerminalOffersNoAnswerChipsAndSaysTheAgentIsBlocked() = runComposeUiTest {
        setContent { Themed { Mobile(PaneView.Terminal) } }
        waitForIdle()

        assertEquals(
            0,
            onAllNodesWithContentDescription("Answer ", substring = true).fetchSemanticsNodes().size,
            "the terminal drew answer chips over a grid that is already showing the dialog",
        )
        assertTrue(
            onAllNodesWithText("blocked", substring = true, ignoreCase = true)
                .fetchSemanticsNodes()
                .isNotEmpty(),
            "the terminal says nothing about the pane being blocked, which is the one thing the " +
                "grid cannot say for itself",
        )
    }

    // Split on a phone draws the terminal surface and nothing else, so it is a terminal for this
    // purpose — the card is not on the screen to be duplicated, and neither is a reason to draw
    // chips over the grid.
    @Test
    fun aSplitOnAPhoneIsATerminalAndOffersNoChipsEither() = runComposeUiTest {
        setContent { Themed { Mobile(PaneView.Split) } }
        waitForIdle()
        assertEquals(
            0,
            onAllNodesWithContentDescription("Answer ", substring = true).fetchSemanticsNodes().size,
        )
    }

    @Test
    fun theDesktopTerminalOffersNoChipsEither() = runComposeUiTest {
        setContent {
            Themed {
                Box(Modifier.size(1280.dp, 800.dp)) {
                    PaneScreenDesktop(
                        pane = blockedPane(),
                        info = INFO,
                        view = PaneView.Terminal,
                        surfaces = NoSurfaces,
                        readOnly = false,
                        onView = {},
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        waitForIdle()
        assertEquals(
            0,
            onAllNodesWithContentDescription("Answer ", substring = true).fetchSemanticsNodes().size,
        )
    }

    @Composable
    private fun Themed(content: @Composable () -> Unit) {
        CompositionLocalProvider(
            LocalTokens provides tokens(),
            LocalConnectionStatus provides ConnectionStatus.Live("full"),
            content = content,
        )
    }

    @Composable
    private fun Mobile(view: PaneView) {
        Box(Modifier.size(411.dp, 891.dp)) {
            PaneScreenMobile(
                pane = blockedPane(),
                info = INFO,
                view = view,
                surfaces = NoSurfaces,
                landscape = false,
                readOnly = false,
                onBack = {},
                onView = {},
                modifier = Modifier.fillMaxSize(),
            )
        }
    }
}
