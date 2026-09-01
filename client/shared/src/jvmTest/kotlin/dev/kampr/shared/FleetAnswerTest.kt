package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.FleetScreen
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.wire.FleetInfo
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Question
import dev.kampr.shared.wire.QuestionOption
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val PANE = "n1/fleet-1"

// A fleet answer is `input` rather than `answer` — there is no `fleet.answer` and there should not
// be — but `input` is `typing` on exactly the same rule, so the board's chips went dead over a
// dead socket in exactly the same way the pane's did.
private val WAITING = Herd(
    nodes = listOf(NodeInfo(id = "n1", name = "n1", kind = "peer", online = true)),
    panes = listOf(
        PaneInfo(
            id = PANE,
            nodeId = "n1",
            fleet = FleetInfo(
                cohort = "c1",
                command = "pacman -Syu",
                state = "waiting",
                question = Question(
                    prompt = "Proceed with installation?",
                    shape = "confirm",
                    options = listOf(QuestionOption("y", "Y"), QuestionOption("n", "n")),
                    defaultKey = "y",
                ),
                startedUnix = 1_700_000_000,
            ),
        ),
    ),
    known = true,
)

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

@OptIn(ExperimentalTestApi::class)
class FleetAnswerTest {
    @Test
    fun aFleetChipOverADeadSocketIsNotPressableAndSaysWhy() = runComposeUiTest {
        var answered: Pair<String, String>? = null
        render(ConnectionStatus.Offline("the hub stopped answering", 4_000)) { pane, key ->
            answered = pane to key
        }
        onNodeWithContentDescription("Y").assertIsNotEnabled()
        onNodeWithContentDescription("Y").performClick()
        assertNull(answered, "a fleet chip over a dead socket answered into the void")
        assertTrue(
            onAllNodesWithText("not connected", substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the board offered an answer over a dead socket and said nothing about it",
        )
    }

    @Test
    fun aFleetChipOverALiveSocketAnswersTheHostItNames() = runComposeUiTest {
        var answered: Pair<String, String>? = null
        render(ConnectionStatus.Live("full")) { pane, key -> answered = pane to key }
        onNodeWithContentDescription("Y").performClick()
        assertEquals(PANE to "y", answered)
    }

    private fun ComposeUiTest.render(status: ConnectionStatus, onAnswer: (String, String) -> Unit) {
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokens(),
                LocalConnectionStatus provides status,
            ) {
                Box(Modifier.size(411.dp, 891.dp)) {
                    FleetScreen(
                        herd = WAITING,
                        breakpoint = Breakpoint.Portrait,
                        onOpenPane = {},
                        onAnswer = onAnswer,
                        onStop = {},
                        onRun = {},
                        canRun = true,
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        waitForIdle()
    }
}
