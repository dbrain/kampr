package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
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
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"
private const val FIRST = "Answer 1, Yes"

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

// The terminal's chip row and the conversation's card send the same `ClientMsg.Answer`, which is
// `typing` on the wire — so both of them went dead on the same socket, and both have to say so.
// The report: *"both conversation and terminal weren't working … it worked slowly after a couple
// of presses"*.
@OptIn(ExperimentalTestApi::class)
class PendingBarTest {
    @Test
    fun aChipOverADeadSocketIsNotPressableAndSaysWhy() = runComposeUiTest {
        var answered: String? = null
        render(ConnectionStatus.Offline("the node stopped answering", 4_000)) { answered = it }
        onNodeWithContentDescription(FIRST).assertIsNotEnabled()
        onNodeWithContentDescription(FIRST).performClick()
        assertNull(answered, "a chip over a dead socket answered into the void")
        assertTrue(
            onAllNodesWithText("not connected", substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the row offered three answers over a dead socket and said nothing about it",
        )
    }

    @Test
    fun aChipOverALiveSocketAnswersWithTheKeyItWasOffered() = runComposeUiTest {
        var answered: String? = null
        render(ConnectionStatus.Live("full")) { answered = it }
        onNodeWithContentDescription(FIRST).performClick()
        assertEquals("1", answered)
    }

    @OptIn(ExperimentalTestApi::class)
    private fun androidx.compose.ui.test.ComposeUiTest.render(
        status: ConnectionStatus,
        onAnswer: (String) -> Unit,
    ) {
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokens(),
                LocalConnectionStatus provides status,
            ) {
                Box(Modifier.size(411.dp, 891.dp)) {
                    PaneScreenMobile(
                        pane = blockedPane(),
                        info = INFO,
                        view = PaneView.Terminal,
                        surfaces = NoSurfaces,
                        landscape = false,
                        readOnly = false,
                        onBack = {},
                        onView = {},
                        onAnswer = onAnswer,
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        waitForIdle()
    }
}
