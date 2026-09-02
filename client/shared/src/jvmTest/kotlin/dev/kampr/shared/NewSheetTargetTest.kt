package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppManage
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.ManageLayer
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.Sheet
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals

private const val MANAGING =
    """{"t":"hello","protocol":1,"node_id":"01JHUB","node_name":"comingclean","build":"test",""" +
        """"role":"full","caps":{"manage":true},"security":{"tier":0,"passkeys":false}}"""

private const val HERD_FRAME =
    """{"t":"herd","nodes":[{"id":"01JHUB","name":"comingclean","kind":"local"},""" +
        """{"id":"01JLOFT","name":"loft","kind":"peer"},""" +
        """{"id":"01JATTIC","name":"attic","kind":"peer","online":false,"detail":"unreachable"}],""" +
        """"panes":[]}"""

private fun KamprStore.take(frame: String) = accept(Wire.decode(frame) ?: error("undecodable: $frame"))

private val HERD = Herd(
    nodes = listOf(
        NodeInfo("01JHUB", "comingclean", "local"),
        NodeInfo("01JLOFT", "loft", "peer"),
        NodeInfo("01JATTIC", "attic", "peer", online = false, detail = "unreachable"),
    ),
    panes = emptyList(),
    known = true,
)

// Reported from a phone: "main screen has a + button but it defaults to the first server and i
// can't seem to switch" — and the only way to create anything on a second machine was to open a
// pane that already lived on it, which is exactly the thing you are trying to make.
@OptIn(ExperimentalTestApi::class)
class NewSheetTargetTest {
    private fun app(): Pair<AppState, CoroutineScope> = withStore().let { (state, scope, _) -> state to scope }

    private fun withStore(): Triple<AppState, CoroutineScope, KamprStore> {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore()
        store.take(MANAGING)
        store.take(HERD_FRAME)
        val state = AppState(scope, store, MemoryPrefs(), null).apply { go(Screen.Herd) }
        return Triple(state, scope, store)
    }

    @Test
    fun theHerdsPlusButtonAimsAtTheNodeItIsConnectedTo() {
        val (state, scope) = app()
        try {
            AppManage(state).openNew(null)
            assertEquals(Sheet.New("01JHUB", null), state.sheet)
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun theSheetCanBeAimedAtAnotherMachineWithoutOpeningAPaneOnIt() = runComposeUiTest {
        val (state, scope) = app()
        try {
            AppManage(state).openNew(null)
            setContent {
                CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                    Box(Modifier.size(411.dp, 914.dp)) {
                        ManageLayer(state, HERD, Breakpoint.Portrait)
                    }
                }
            }
            waitForIdle()
            onNodeWithContentDescription("Change machine, currently comingclean").performSemanticsAction(SemanticsActions.OnClick)
            waitForIdle()
            onNodeWithContentDescription("Create on loft").performSemanticsAction(SemanticsActions.OnClick)
            waitForIdle()
            assertEquals(
                Sheet.New("01JLOFT", null),
                state.sheet,
                "the sheet stayed aimed at the machine it opened on",
            )
        } finally {
            scope.cancel()
        }
    }

    // A machine the herd says is unreachable cannot run anything, so it is listed and refused
    // rather than offered and failing.
    @Test
    fun anOfflineMachineIsNotSomethingToCreateOn() = runComposeUiTest {
        val (state, scope) = app()
        try {
            AppManage(state).openNew(null)
            setContent {
                CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                    Box(Modifier.size(411.dp, 914.dp)) {
                        ManageLayer(state, HERD, Breakpoint.Portrait)
                    }
                }
            }
            waitForIdle()
            onNodeWithContentDescription("Change machine, currently comingclean").performSemanticsAction(SemanticsActions.OnClick)
            waitForIdle()
            assertEquals(
                1,
                onAllNodesWithContentDescription("attic, unreachable").fetchSemanticsNodes().size,
                "an unreachable machine has to be listed, and listed as unreachable",
            )
            assertEquals(Sheet.New("01JHUB", null), state.sheet)
        } finally {
            scope.cancel()
        }
    }

    // End to end through the layer that owns the seam: press the button, take the node's ack off
    // the socket, take the patch that follows, and the operator is looking at what they made.
    @Test
    fun creatingAWorkspaceLandsTheOperatorInIt() = runComposeUiTest {
        val (state, scope, store) = withStore()
        try {
            AppManage(state).openNew(null)
            setContent {
                CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                    Box(Modifier.size(411.dp, 914.dp)) {
                        ManageLayer(state, HERD, Breakpoint.Portrait)
                    }
                }
            }
            waitForIdle()
            onNodeWithContentDescription("Create workspace").performSemanticsAction(SemanticsActions.OnClick)
            waitForIdle()

            store.take("""{"t":"managed","op":"workspace.create","ok":true,"id":"01JHUB/w7"}""")
            waitForIdle()
            assertEquals(Screen.Herd, state.screen, "there is no pane to open until the patch lands")

            store.take(
                """{"t":"herd","nodes":[{"id":"01JHUB","name":"comingclean","kind":"local"}],""" +
                    """"panes":[{"id":"01JHUB/w7:p1","node_id":"01JHUB","workspace_id":"01JHUB/w7","updated_at":"7"}]}"""
            )
            waitForIdle()
            assertEquals("01JHUB/w7:p1", (state.screen as Screen.Pane).paneId)
        } finally {
            scope.cancel()
        }
    }
}
