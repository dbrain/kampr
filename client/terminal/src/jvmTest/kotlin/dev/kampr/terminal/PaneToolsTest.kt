package dev.kampr.terminal

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val REVIEW = "Review this pane row by row"
private const val ATTACH = "Attach a file for this pane"

private object ReadOnlyIo : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
    override val readOnly = true
}

// The actions sheet is composed at the app root, a long way from the grid, so these are the
// surfaces' own tools slot beside the terminal they act on — the shape `ManageLayer` hands them.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.terminalAndItsTools(io: PaneIo = HushIo, onScreen: Boolean = true): () -> Int {
    val surfaces = TerminalSurfaces()
    val pane = Phone.shell()
    var closed by mutableStateOf(0)
    setContent {
        CompositionLocalProvider(LocalTokens provides Phone.tokens(), LocalPaneIo provides io) {
            Column(Modifier.fillMaxSize()) {
                Row { surfaces.Tools(pane.id) { closed++ } }
                if (onScreen) surfaces.Terminal(pane, null, Modifier.weight(1f))
            }
        }
    }
    waitForIdle()
    return { closed }
}

@OptIn(ExperimentalTestApi::class)
class PaneToolsTest {
    @Test
    fun reviewFromTheActionsSheetClosesItAndStartsReadingTheGrid() = runComposeUiTest {
        val closed = terminalAndItsTools()
        onNodeWithContentDescription(REVIEW).performClick()
        waitForIdle()
        assertEquals(1, closed(), "the sheet stayed up over the rows about to be read")
        onNodeWithContentDescription("Leave review").assertExists()
    }

    @Test
    fun theTerminalOffersToAttachSomethingWhereThereIsAPickerToRaise() = runComposeUiTest {
        terminalAndItsTools()
        onNodeWithContentDescription(ATTACH).assertExists()
    }

    @Test
    fun aReadOnlyDeviceIsOfferedNothingToAttach() = runComposeUiTest {
        terminalAndItsTools(io = ReadOnlyIo)
        assertTrue(
            onAllNodesWithContentDescription(ATTACH).fetchSemanticsNodes().isEmpty(),
            "a device that cannot type was offered a way to type a path in",
        )
        onNodeWithContentDescription(REVIEW).assertExists()
    }

    @Test
    fun aPaneShowingOnlyItsTranscriptIsOfferedNoTerminalTools() = runComposeUiTest {
        terminalAndItsTools(onScreen = false)
        for (tool in listOf(REVIEW, ATTACH)) {
            assertTrue(
                onAllNodesWithContentDescription(tool).fetchSemanticsNodes().isEmpty(),
                "'$tool' was offered with no grid on screen to do it to",
            )
        }
    }
}
