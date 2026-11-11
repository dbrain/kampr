package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.InternalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.navigationevent.compose.LocalNavigationEventDispatcherOwner
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.NewSheet
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local")

// The sheet is a ladder, and its header grows a Back control the moment a step is open. The system
// gesture owes the same move: it walks the ladder before it closes the sheet, or a mistap on
// "Named session" is answered by the whole sheet vanishing.
@OptIn(ExperimentalTestApi::class, InternalComposeUiApi::class)
class NewSheetBackTest {
    private fun ComposeUiTest.sheet(window: SystemBackWindow, height: Dp = 900.dp, onDismiss: () -> Unit = {}) {
        setContent {
            CompositionLocalProvider(
                LocalTokens provides phoneTokens(),
                LocalNavigationEventDispatcherOwner provides window,
            ) {
                Box(Modifier.size(420.dp, height)) {
                    NewSheet(
                        breakpoint = Breakpoint.Portrait,
                        node = NODE,
                        pane = null,
                        nodes = listOf(NODE),
                        caps = ServerMsg.NodeCaps(node = NODE.id, agentKinds = listOf("claude"), sessions = emptyList()),
                        outcome = null,
                        onManage = {},
                        onNode = {},
                        onNodePicker = {},
                        onDismiss = onDismiss,
                        onRefreshCaps = {},
                    )
                }
            }
        }
        waitForIdle()
    }

    @Test
    fun backInsideTheSheetGoesUpAStepBeforeItClosesTheSheet() = runComposeUiTest {
        val window = SystemBackWindow()
        var dismissed = false
        sheet(window) { dismissed = true }
        assertFalse(window.claimed, "the sheet's first step has nothing of its own to go back to")

        onNodeWithContentDescription("Named session, Its own server")
            .performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        assertTrue(window.claimed, "a step with a Back control in its header does not claim the gesture")

        window.press()
        waitForIdle()
        assertFalse(dismissed, "back off a step closed the whole sheet")
        onNodeWithContentDescription("Named session, Its own server").assertExists()
    }

    // The menu is the rung a step goes back to, so it comes back where the operator left it rather
    // than wherever the step's own scroll happened to stop.
    @Test
    fun backToTheMenuRestoresWhereTheMenuWasScrolled() = runComposeUiTest {
        val window = SystemBackWindow()
        sheet(window, height = 420.dp)
        val menu = onNode(VERTICAL)
        menu.performSemanticsAction(SemanticsActions.ScrollBy) { it(0f, 10_000f) }
        waitForIdle()
        val before = onNode(VERTICAL).fetchSemanticsNode().config[SemanticsProperties.VerticalScrollAxisRange].value()
        assertTrue(before > 0f, "the menu scrolled")

        onNodeWithContentDescription("Named session, Its own server")
            .performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        window.press()
        waitForIdle()

        val after = onNode(VERTICAL).fetchSemanticsNode().config[SemanticsProperties.VerticalScrollAxisRange].value()
        assertEquals(before, after)
    }
}

private val VERTICAL = SemanticsMatcher("scrolls vertically") { node ->
    node.config.getOrNull(SemanticsProperties.VerticalScrollAxisRange)?.let { it.maxValue() > 0f } == true
}
