package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.InternalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.backhandler.LocalCompatNavigationEventDispatcherOwner
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.NewSheet
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private val NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local")

// The sheet is a ladder, and its header grows a Back control the moment a step is open. The system
// gesture owes the same move: it walks the ladder before it closes the sheet, or a mistap on
// "Named session" is answered by the whole sheet vanishing.
@OptIn(ExperimentalTestApi::class, InternalComposeUiApi::class)
class NewSheetBackTest {
    @Test
    fun backInsideTheSheetGoesUpAStepBeforeItClosesTheSheet() = runComposeUiTest {
        val window = SystemBackWindow()
        var dismissed = false
        setContent {
            CompositionLocalProvider(
                LocalTokens provides phoneTokens(),
                LocalCompatNavigationEventDispatcherOwner provides window,
            ) {
                Box(Modifier.size(420.dp, 900.dp)) {
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
                        onDismiss = { dismissed = true },
                        onRefreshCaps = {},
                    )
                }
            }
        }
        waitForIdle()
        assertFalse(window.claimed, "the sheet's first step has nothing of its own to go back to")

        onNodeWithContentDescription("Named session, its own server")
            .performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        assertTrue(window.claimed, "a step with a Back control in its header does not claim the gesture")

        window.press()
        waitForIdle()
        assertFalse(dismissed, "back off a step closed the whole sheet")
        onNodeWithContentDescription("Named session, its own server").assertExists()
    }
}
