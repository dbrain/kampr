package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.NewSheet
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private val DESKED = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local")
private val DESKLESS = DESKED.copy(online = false)
private val FROM = PaneInfo(
    id = "01JNODE/w3:p2",
    nodeId = "01JNODE",
    workspaceId = "01JNODE/w3",
    workspace = "kampr",
    label = "shell",
    cwd = "/home/dbrain/dev/kampr",
)
private const val DESK_TOGGLE = "Also show on the desk"
private const val START = "Start Kampr session"

private class NativeSheet(val node: NodeInfo = DESKED, val pane: PaneInfo? = null) {
    val sent = mutableListOf<ManageOp>()
    val created = mutableListOf<String>()
    var dismissed = false
    val outcome = mutableStateOf<ServerMsg.Managed?>(null)
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.open(sheet: NativeSheet) {
    setContent {
        CompositionLocalProvider(LocalTokens provides phoneTokens()) {
            Box(Modifier.size(420.dp, 1200.dp)) {
                NewSheet(
                    breakpoint = Breakpoint.Portrait,
                    node = sheet.node,
                    pane = sheet.pane,
                    nodes = listOf(sheet.node),
                    caps = null,
                    outcome = sheet.outcome.value,
                    onManage = { sheet.sent += it },
                    onNode = {},
                    onNodePicker = {},
                    onDismiss = { sheet.dismissed = true },
                    onCreated = { sheet.created += it },
                )
            }
        }
    }
    onNodeWithContentDescription("Kampr session,", substring = true).performSemanticsAction(SemanticsActions.OnClick)
    waitForIdle()
}

@OptIn(ExperimentalTestApi::class)
class NewSheetNativeTest {
    @Test
    fun anEmptyCommandIsTheLoginShellAndTheAckOpensWhatItMade() = runComposeUiTest {
        val sheet = NativeSheet()
        open(sheet)
        onNodeWithContentDescription(START).performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.NativeCreate(DESKED.id)), sheet.sent)

        sheet.outcome.value = ServerMsg.Managed(op = "native.create", ok = true, id = "01JNODE/n:01ja")
        waitForIdle()
        assertEquals(listOf("01JNODE/n:01ja"), sheet.created)
        assertTrue(sheet.dismissed)
    }

    @Test
    fun theCommandAndDirectoryAreSentAsTyped() = runComposeUiTest {
        val sheet = NativeSheet(pane = FROM)
        open(sheet)
        onNodeWithContentDescription("command").performTextInput("  cargo test && make ")
        waitForIdle()
        onNodeWithContentDescription(START).performClick()
        waitForIdle()
        assertEquals(
            listOf<ManageOp>(ManageOp.NativeCreate(DESKED.id, command = "cargo test && make", cwd = FROM.cwd)),
            sheet.sent,
        )
    }

    @Test
    fun showingItOnTheDeskSplitsBesideThePaneTheSheetWasOpenedFrom() = runComposeUiTest {
        val sheet = NativeSheet(pane = FROM)
        open(sheet)
        onNodeWithContentDescription(DESK_TOGGLE, substring = true).performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        onNodeWithContentDescription(START).performClick()
        waitForIdle()
        assertEquals(
            listOf<ManageOp>(ManageOp.NativeCreate(DESKED.id, cwd = FROM.cwd, desk = true, at = FROM.id)),
            sheet.sent,
        )
    }

    @Test
    fun aMachineWithNoDeskIsNotOfferedOne() = runComposeUiTest {
        val sheet = NativeSheet(node = DESKLESS)
        open(sheet)
        onNodeWithContentDescription(DESK_TOGGLE, substring = true).assertDoesNotExist()
        onNodeWithContentDescription(START).performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.NativeCreate(DESKLESS.id)), sheet.sent)
    }

    @Test
    fun aRefusalStaysOnTheSheetAndSaysWhy() = runComposeUiTest {
        val sheet = NativeSheet()
        open(sheet)
        onNodeWithContentDescription(START).performClick()
        waitForIdle()
        sheet.outcome.value = ServerMsg.Managed(
            op = "native.create",
            ok = false,
            id = null,
            code = "bad_request",
            message = "no such directory",
        )
        waitForIdle()
        assertFalse(sheet.dismissed)
        assertEquals(emptyList(), sheet.created)
        onNodeWithContentDescription("no such directory").assertExists()
    }
}
