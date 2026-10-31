package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.hasDesk
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.NewSheet
import dev.kampr.shared.ui.NodeQuiet
import dev.kampr.shared.ui.nodeFacts
import dev.kampr.shared.ui.quickSessionOp
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// The operator: *"let's go full kampr and herdr is the 'help you migrate/test it out' piece"*. A
// node with herdr off is a node, not a node missing something: nothing herdr-shaped is offered on it
// and nothing on screen mentions herdr at all.
private val KAMPR_ONLY = NodeInfo(
    id = "01JNODE",
    name = "comingclean",
    kind = "local",
    online = false,
    reachable = true,
    native = true,
    herdr = false,
)
private val WITH_HERDR = KAMPR_ONLY.copy(online = true, herdr = true, herdrVersion = "0.9.5")
private val FROM = PaneInfo(
    id = "01JNODE/w3:p2",
    nodeId = "01JNODE",
    workspaceId = "01JNODE/w3",
    workspace = "kampr",
    label = "shell",
    cwd = "/home/dbrain/dev/kampr",
)
private const val START = "Start Kampr session"

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.sheet(node: NodeInfo, pane: PaneInfo? = null, sent: MutableList<ManageOp> = mutableListOf()) = setContent {
    phone {
        NewSheet(
            breakpoint = Breakpoint.Portrait,
            node = node,
            pane = pane,
            nodes = listOf(node),
            caps = null,
            outcome = null,
            onManage = { sent += it },
            onNode = {},
            onNodePicker = {},
            onDismiss = {},
        )
    }
}

@Composable
private fun phone(content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides phoneTokens()) {
        Box(Modifier.size(420.dp, 1600.dp)) { content() }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.saysNothingAboutHerdr() {
    onAllNodes(
        hasText("herdr", substring = true, ignoreCase = true) or
            hasContentDescription("herdr", substring = true, ignoreCase = true),
        useUnmergedTree = true,
    ).assertCountEquals(0)
}

@OptIn(ExperimentalTestApi::class)
class HerdrOffTest {
    @Test
    fun aNodeWithHerdrOffIsOfferedKamprSessionsAndNothingOfHerdrs() = runComposeUiTest {
        sheet(KAMPR_ONLY, FROM)
        waitForIdle()
        for (herdrs in listOf("Workspace,", "Tab,", "Split this pane,", "Worktree,", "Named session,", "Start a")) {
            onAllNodesWithContentDescription(herdrs, substring = true).assertCountEquals(0)
        }
        onNodeWithContentDescription("Kampr session,", substring = true).assertExists()
        saysNothingAboutHerdr()
    }

    @Test
    fun aKamprSessionOnANodeWithHerdrOffIsNotOfferedTheDesk() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        sheet(KAMPR_ONLY, FROM, sent)
        onNodeWithContentDescription("Kampr session,", substring = true).performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
        onAllNodesWithContentDescription("Also show on the desk", substring = true).assertCountEquals(0)
        saysNothingAboutHerdr()
        onNodeWithContentDescription(START).performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.NativeCreate(KAMPR_ONLY.id, cwd = FROM.cwd)), sent)
    }

    // Herdr on is the extra: the sheet still leads with a Kampr session, and its own primary button
    // starts one.
    @Test
    fun theSheetLeadsWithAKamprSessionEvenWhereHerdrIsOn() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        sheet(WITH_HERDR, sent = sent)
        waitForIdle()
        val kampr = onNodeWithContentDescription("Kampr session,", substring = true).fetchSemanticsNode().boundsInRoot
        val workspace = onNodeWithContentDescription("Workspace,", substring = true).fetchSemanticsNode().boundsInRoot
        assertTrue(kampr.top < workspace.top, "a herdr workspace is offered above a Kampr session")
        onNodeWithContentDescription(START).performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.NativeCreate(WITH_HERDR.id, desk = true)), sent)
    }

    @Test
    fun aMachineWithHerdrOffIsNotDescribedByWhatItLacks() = runComposeUiTest {
        setContent { phone { NodeQuiet(KAMPR_ONLY) } }
        waitForIdle()
        saysNothingAboutHerdr()
        val facts = nodeFacts(KAMPR_ONLY.copy(herdrVersion = "0.9.5"), withStatus = true)
        assertFalse(facts.any { it.contains("herdr", ignoreCase = true) }, "$facts")
        assertTrue("online" in nodeFacts(KAMPR_ONLY, withStatus = true))
    }

    @Test
    fun aNodeWithHerdrOffHasNoDeskAndItsPlusKeepsTheSessionOffIt() {
        assertFalse(KAMPR_ONLY.copy(herdrVersion = "0.9.5").hasDesk)
        assertEquals(ManageOp.NativeCreate(KAMPR_ONLY.id, desk = false), quickSessionOp(KAMPR_ONLY.copy(herdrVersion = "0.9.5")))
        assertTrue(WITH_HERDR.hasDesk)
    }
}
