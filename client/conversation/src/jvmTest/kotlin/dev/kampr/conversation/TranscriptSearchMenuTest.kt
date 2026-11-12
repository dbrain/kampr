package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.v2.runComposeUiTest
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.MenuRow
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val MENU = "Search this conversation's transcript"
private const val FIELD = "Search the transcript"

private object Beneath : PaneSurfaces {
    @Composable override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)

    @Composable override fun Tools(paneId: String, onUsed: () -> Unit) = MenuRow("Zoom · fit", onUsed)
}

// The shape `ManageLayer` hands the surfaces: the actions sheet's tools slot, a long way from the
// conversation it acts on.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.conversationAndItsTools(onScreen: Boolean = true): () -> Int {
    val surfaces = ConversationSurfaces(Beneath)
    val (_, pane) = demoPane(RICH_CONVO)
    var closed by mutableStateOf(0)
    setContent {
        CompositionLocalProvider(LocalTokens provides tokensFor(SoftTheme, TypeScale.Phone), LocalPaneIo provides RecordingIo) {
            Column(Modifier.fillMaxSize()) {
                Column { surfaces.Tools(pane.id) { closed++ } }
                if (onScreen) surfaces.Conversation(pane, demoInfo(), Modifier.weight(1f))
            }
        }
    }
    waitForIdle()
    return { closed }
}

@OptIn(ExperimentalTestApi::class)
class TranscriptSearchMenuTest {
    private fun ComposeUiTest.has(description: String) = onAllNodesWithContentDescription(description).fetchSemanticsNodes().isNotEmpty()

    // The report: the transcript bar was large and said little. It is gone until a search is asked
    // for, and the search is asked for where every other pane action is.
    @Test
    fun searchOpensFromThePaneActionsAndTheBarIsOnlyThereWhileItIs() = runComposeUiTest {
        val closed = conversationAndItsTools()
        assertTrue(!has(FIELD), "a search field stood over a transcript nobody was searching")
        assertTrue(onAllNodesWithText("turns", substring = true).fetchSemanticsNodes().isEmpty(), "the turn-count bar is still there")

        onNodeWithContentDescription(MENU).performClick()
        waitForIdle()
        assertEquals(1, closed(), "the sheet stayed up over the field about to be typed into")
        assertTrue(has(FIELD), "the search did not open")

        onNodeWithContentDescription("Close search").performClick()
        waitForIdle()
        assertTrue(!has(FIELD), "closing the search left its bar behind")
    }

    @Test
    fun aConversationNotOnScreenOffersNoSearch() = runComposeUiTest {
        conversationAndItsTools(onScreen = false)
        assertTrue(!has(MENU))
    }

    // Every composition root wraps the terminal's surfaces in these, so a tool the terminal offers
    // reaches the sheet only through here.
    @Test
    fun theSurfacesBeneathKeepTheirTools() = runComposeUiTest {
        conversationAndItsTools()
        assertTrue(has("Zoom · fit"), "the terminal's tools never reached the actions sheet")
    }
}
