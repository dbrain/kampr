package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.v2.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.NewSheet
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val CLAUDE_NODE = NodeInfo(id = "01JNODE", name = "comingclean", kind = "local", native = true, claude = true)
private const val CLAUDE = "Claude (no terminal),"

private class ClaudeSheet {
    val sent = mutableListOf<ManageOp>()
    val asked = mutableListOf<Pair<ManageOp, String>>()
    val created = mutableListOf<String>()
    var dismissed = false
    val outcome = mutableStateOf<ServerMsg.Managed?>(null)
    val status = mutableStateOf<ConnectionStatus>(ConnectionStatus.Live("full"))
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.show(node: NodeInfo, sheet: ClaudeSheet) = setContent {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalConnectionStatus provides sheet.status.value) {
        Box(Modifier.size(420.dp, 1200.dp)) {
            NewSheet(
                breakpoint = Breakpoint.Portrait,
                node = node,
                pane = null,
                nodes = listOf(node),
                caps = null,
                outcome = sheet.outcome.value,
                onManage = { sheet.sent += it },
                onAsk = { op, rid -> sheet.asked += op to rid },
                onNode = {},
                onNodePicker = {},
                onDismiss = { sheet.dismissed = true },
                onCreated = { sheet.created += it },
            )
        }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.open(sheet: ClaudeSheet) {
    show(CLAUDE_NODE, sheet)
    onNodeWithContentDescription(CLAUDE, substring = true).performSemanticsAction(SemanticsActions.OnClick)
    waitForIdle()
}

private fun managed(json: String) = Wire.decode(json) as ServerMsg.Managed

private fun listing(rid: String, vararg titles: String) = managed(
    """{"t":"managed","op":"claude.resumable","ok":true,"rid":"$rid","resumable":[${
        titles.joinToString(",") { """{"id":"$it","cwd":"/w","title":"$it"}""" }
    }]}""",
)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.typeDirectory(sheet: ClaudeSheet, text: String): String {
    onNodeWithContentDescription("Directory").performTextInput(text)
    mainClock.advanceTimeBy(1_000)
    waitForIdle()
    return sheet.asked.last().second
}

// docs/21 Phase 5: "the client's new conversation", and resuming one from the transcripts on disk.
@OptIn(ExperimentalTestApi::class)
class NewSheetClaudeTest {
    @Test
    fun theCardIsOfferedOnlyByANodeThatSaysItCanStartOne() {
        for ((claude, offered) in listOf(true to true, false to false, null to false)) {
            runComposeUiTest {
                show(CLAUDE_NODE.copy(claude = claude), ClaudeSheet())
                waitForIdle()
                val shown = onAllNodesWithContentDescription(CLAUDE, substring = true).fetchSemanticsNodes().isNotEmpty()
                assertEquals(offered, shown, "claude=$claude")
            }
        }
    }

    @Test
    fun theDirectoryAndPromptAreSentAndTheAckOpensWhatItMade() = runComposeUiTest {
        val sheet = ClaudeSheet()
        open(sheet)
        onNodeWithContentDescription("Directory").performTextInput(" /home/dbrain/dev/kampr ")
        onNodeWithContentDescription("First prompt").performTextInput("tidy the parser")
        waitForIdle()
        onNodeWithContentDescription("Start conversation").performClick()
        waitForIdle()
        assertEquals(
            ManageOp.ClaudeCreate("01JNODE", "/home/dbrain/dev/kampr", "tidy the parser"),
            sheet.sent.filterIsInstance<ManageOp.ClaudeCreate>().single(),
        )
        sheet.outcome.value = managed("""{"t":"managed","op":"claude.create","ok":true,"id":"01JNODE/c:c1"}""")
        waitForIdle()
        assertEquals(listOf("01JNODE/c:c1"), sheet.created)
        assertTrue(sheet.dismissed)
    }

    @Test
    fun theConversationsOnDiskAreListedForTheDirectoryAndOneIsResumed() = runComposeUiTest {
        val sheet = ClaudeSheet()
        open(sheet)
        val rid = typeDirectory(sheet, "/w")
        assertEquals(ManageOp.ClaudeResumable("01JNODE", "/w"), sheet.asked.last().first)

        sheet.outcome.value = managed(
            """{"t":"managed","op":"claude.resumable","ok":true,"rid":"$rid","resumable":[{"id":"u-1","cwd":"/w","title":"fix the build"}]}""",
        )
        waitForIdle()
        assertTrue(!sheet.dismissed, "a listing closed the sheet")
        onNodeWithContentDescription("Resume fix the build").performClick()
        waitForIdle()
        assertEquals(ManageOp.ClaudeResume("01JNODE", "u-1"), sheet.sent.last())

        sheet.outcome.value = managed("""{"t":"managed","op":"claude.resume","ok":true,"id":"01JNODE/c:c2"}""")
        waitForIdle()
        assertEquals(listOf("01JNODE/c:c2"), sheet.created)
    }

    @Test
    fun aRefusedListingSaysWhyRatherThanThatThereIsNothing() = runComposeUiTest {
        val sheet = ClaudeSheet()
        open(sheet)
        val rid = typeDirectory(sheet, "/w")
        sheet.outcome.value = managed(
            """{"t":"managed","op":"claude.resumable","ok":false,"rid":"$rid","code":"bad_request","message":"/w is not a directory"}""",
        )
        waitForIdle()
        onNodeWithText("/w is not a directory").assertExists()
        onNodeWithText("No conversation to resume here.").assertDoesNotExist()
        assertTrue(!sheet.dismissed)
    }

    @Test
    fun aListingForADirectoryNoLongerTypedIsDropped() = runComposeUiTest {
        val sheet = ClaudeSheet()
        open(sheet)
        val first = typeDirectory(sheet, "/w")
        val second = typeDirectory(sheet, "/x")
        assertEquals(ManageOp.ClaudeResumable("01JNODE", "/w/x"), sheet.asked.last().first)

        sheet.outcome.value = listing(second, "current")
        waitForIdle()
        sheet.outcome.value = listing(first, "stale")
        waitForIdle()
        onNodeWithContentDescription("Resume current").assertExists()
        onNodeWithContentDescription("Resume stale").assertDoesNotExist()
    }

    @Test
    fun anAnswerForAnotherDirectoryIsNotShownWhileTheFreshAskIsPending() = runComposeUiTest {
        val sheet = ClaudeSheet()
        open(sheet)
        sheet.outcome.value = listing(typeDirectory(sheet, "/w"), "stale")
        waitForIdle()
        onNodeWithContentDescription("Resume stale").assertExists()
        typeDirectory(sheet, "/x")
        onNodeWithContentDescription("Resume stale").assertDoesNotExist()
        onNodeWithText("Looking for conversations…").assertExists()

        val rid = typeDirectory(sheet, "/y")
        sheet.outcome.value = managed("""{"t":"managed","op":"claude.resumable","ok":false,"rid":"$rid","code":"bad_request","message":"not a directory"}""")
        waitForIdle()
        onNodeWithText("not a directory").assertExists()
        typeDirectory(sheet, "/z")
        onNodeWithText("not a directory").assertDoesNotExist()
        onNodeWithText("Looking for conversations…").assertExists()
    }

    // An ask that went out as the socket dropped is never answered.
    @Test
    fun aReconnectAsksAgainSoTheSpinnerDoesNotStayUp() = runComposeUiTest {
        val sheet = ClaudeSheet()
        open(sheet)
        val lost = typeDirectory(sheet, "/w")
        sheet.status.value = ConnectionStatus.Offline("dropped", 1_000)
        mainClock.advanceTimeBy(1_000)
        waitForIdle()
        sheet.status.value = ConnectionStatus.Live("full")
        mainClock.advanceTimeBy(1_000)
        waitForIdle()
        val (op, rid) = sheet.asked.last()
        assertEquals(ManageOp.ClaudeResumable("01JNODE", "/w"), op)
        assertTrue(rid != lost, "the reconnect asked nothing")

        sheet.outcome.value = listing(rid, "fix the build")
        waitForIdle()
        onNodeWithText("Looking for conversations…").assertDoesNotExist()
        onNodeWithContentDescription("Resume fix the build").assertExists()
    }
}
