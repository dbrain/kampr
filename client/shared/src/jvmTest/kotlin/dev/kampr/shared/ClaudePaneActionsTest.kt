package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.v2.runComposeUiTest
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.LocalConnectionStatus
import dev.kampr.shared.ui.PaneActionsSheet
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val HEADLESS_PANE = PaneInfo(id = "01JNODE/c:c1", nodeId = "01JNODE", agent = "claude", cwd = "/w")
private val TERMINAL_CLAUDE = PaneInfo(id = "01JNODE/n:01ja", nodeId = "01JNODE", agent = "claude", provider = "native", sessionId = "01ja")
private val TERMINAL_SHELL = TERMINAL_CLAUDE.copy(agent = null)

private class ClaudeActions {
    val sent = mutableListOf<ManageOp>()
    val created = mutableListOf<String>()
    var dismissed = false
    val outcome = mutableStateOf<ServerMsg.Managed?>(null)
    val status = mutableStateOf<ConnectionStatus>(ConnectionStatus.Live("full"))
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.sheet(pane: PaneInfo, headless: Boolean, actions: ClaudeActions) = setContent {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalConnectionStatus provides actions.status.value) {
        Box(Modifier.fillMaxSize()) {
            PaneActionsSheet(
                Breakpoint.Portrait,
                pane,
                actions.outcome.value,
                { actions.sent += it },
                { actions.dismissed = true },
                headless = headless,
                onCreated = { actions.created += it },
            )
        }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.shown(text: String) = onAllNodesWithText(text).fetchSemanticsNodes().isNotEmpty()

private fun managed(op: String, ok: Boolean, extra: String = "") = Wire.decode("""{"t":"managed","op":"$op","ok":$ok$extra}""") as ServerMsg.Managed

private const val BUSY = "Claude is busy: moving it now stops the turn in progress."

private data class Hand(val pane: PaneInfo, val row: String, val confirm: String, val op: ManageOp, val forced: ManageOp)

private val HANDS = listOf(
    Hand(HEADLESS_PANE, "Open in terminal", "Stop and open in terminal", ManageOp.ClaudeToTerminal(HEADLESS_PANE.id), ManageOp.ClaudeToTerminal(HEADLESS_PANE.id, interrupt = true)),
    Hand(TERMINAL_CLAUDE, "Continue without terminal", "Stop and continue", ManageOp.ClaudeFromTerminal(TERMINAL_CLAUDE.id), ManageOp.ClaudeFromTerminal(TERMINAL_CLAUDE.id, interrupt = true)),
)

// docs/21: a Claude at rest moves on one press; a busy one is refused with what would stop, and
// that sentence is the confirmation. The ack opens where the conversation went.
@OptIn(ExperimentalTestApi::class)
class ClaudePaneActionsTest {
    @Test
    fun aClaudeAtRestMovesOnOnePressAndOpensWhereItWent() {
        for (hand in HANDS) {
            runComposeUiTest {
                val actions = ClaudeActions()
                sheet(hand.pane, true, actions)
                onNodeWithText(hand.row).performClick()
                waitForIdle()
                assertEquals(listOf(hand.op), actions.sent, "$hand")

                actions.outcome.value = managed(hand.op.op, true, ""","id":"01JNODE/n:k1"""")
                waitForIdle()
                assertEquals(listOf("01JNODE/n:k1"), actions.created, "$hand")
                assertTrue(actions.dismissed, "$hand")
            }
        }
    }

    @Test
    fun aBusyClaudeIsStoppedOnlyOnceTheOperatorReadsWhatStops() {
        for (hand in HANDS) {
            runComposeUiTest {
                val actions = ClaudeActions()
                sheet(hand.pane, true, actions)
                onNodeWithText(hand.row).performClick()
                waitForIdle()
                actions.outcome.value = managed(hand.op.op, false, ""","code":"busy","message":"$BUSY"""")
                waitForIdle()
                assertTrue(shown(BUSY), "the confirmation does not say what stops: $hand")
                assertEquals(listOf(hand.op), actions.sent, "$hand")
                onNodeWithText(hand.confirm).performClick()
                waitForIdle()
                assertEquals(listOf(hand.op, hand.forced), actions.sent, "$hand")
            }
        }
    }

    @Test
    fun aHeadlessConversationEndsOnceConfirmed() = runComposeUiTest {
        val actions = ClaudeActions()
        sheet(HEADLESS_PANE, true, actions)
        assertTrue(!shown("Fill the tab"), "a conversation with no terminal was offered a herdr pane's verbs")
        onNodeWithText("End session…").performClick()
        waitForIdle()
        onNodeWithText("End session").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.ClaudeEnd(HEADLESS_PANE.id)), actions.sent)
    }

    // A second hand-over of the same conversation forks it, so the row waits out its ack.
    @Test
    fun aHandOverInFlightIsNotSentAgainAndItsRowComesBackOnTheAck() {
        for (hand in HANDS) {
            runComposeUiTest {
                val actions = ClaudeActions()
                sheet(hand.pane, true, actions)
                onNodeWithText(hand.row).performClick()
                waitForIdle()
                assertEquals(listOf(hand.op), actions.sent, "$hand")

                onNodeWithText(hand.row).assertIsNotEnabled()
                onNodeWithContentDescription("Waiting for the node", substring = true).assertExists()
                onNodeWithText(hand.row).performClick()
                waitForIdle()
                assertEquals(listOf(hand.op), actions.sent, "$hand")

                actions.outcome.value = managed(hand.op.op, false, ""","code":"bad_request","message":"no stored conversation"""")
                waitForIdle()
                assertTrue(shown("no stored conversation"), "$hand")
                onNodeWithText(hand.row).assertIsEnabled()
                onNodeWithText(hand.row).performClick()
                waitForIdle()
                assertEquals(listOf(hand.op, hand.op), actions.sent, "$hand")
            }
        }
    }

    // An op that went out as the socket dropped is never answered; the node refuses a repeat.
    @Test
    fun aHandOverInFlightGivesItsRowBackWhenTheSocketDrops() = runComposeUiTest {
        val actions = ClaudeActions()
        sheet(HEADLESS_PANE, true, actions)
        onNodeWithText("Open in terminal").performClick()
        waitForIdle()
        onNodeWithText("Open in terminal").assertIsNotEnabled()

        actions.status.value = ConnectionStatus.Offline("gone", 1_000)
        waitForIdle()
        onNodeWithText("Open in terminal").assertIsEnabled()
    }

    @Test
    fun onlyATerminalClaudeOnANodeThatCanContinuesWithoutItsTerminal() {
        data class Case(val pane: PaneInfo, val headless: Boolean, val offered: Boolean)
        for (case in listOf(Case(TERMINAL_CLAUDE, true, true), Case(TERMINAL_CLAUDE, false, false), Case(TERMINAL_SHELL, true, false))) {
            runComposeUiTest {
                val actions = ClaudeActions()
                sheet(case.pane, case.headless, actions)
                assertEquals(case.offered, shown("Continue without terminal"), "$case")
            }
        }
    }
}
