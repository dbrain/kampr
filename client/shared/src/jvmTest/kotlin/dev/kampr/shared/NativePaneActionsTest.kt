package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.MenuAnchor
import dev.kampr.shared.ui.PaneActionsSheet
import dev.kampr.shared.ui.PaneMenu
import dev.kampr.shared.ui.PaneRow
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NativeExit
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val SESSION = PaneInfo(
    id = "01JNODE/n:01ja",
    nodeId = "01JNODE",
    label = "build",
    cwd = "/home/dbrain/dev/kampr",
    provider = "native",
    sessionId = "01ja",
    detached = true,
)
private val ENDED = SESSION.copy(exited = NativeExit(signal = 9))

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.menu(pane: PaneInfo, desk: Boolean, sent: MutableList<ManageOp>) = setContent {
    CompositionLocalProvider(LocalTokens provides phoneTokens()) {
        Box(Modifier.size(900.dp, 600.dp)) {
            PaneMenu(Breakpoint.Desktop, pane, MenuAnchor(120.dp, 120.dp), null, { sent += it }, {}, {}, desk = desk)
        }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.sheet(pane: PaneInfo, desk: Boolean, sent: MutableList<ManageOp>) = setContent {
    CompositionLocalProvider(LocalTokens provides phoneTokens()) {
        Box(Modifier.fillMaxSize()) {
            PaneActionsSheet(Breakpoint.Portrait, pane, null, { sent += it }, {}, desk = desk)
        }
    }
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.texts(): List<String> = onAllNodesWithText("", substring = true)
    .fetchSemanticsNodes()
    .mapNotNull { node -> node.config.getOrElseNullable(SemanticsProperties.Text) { null }?.joinToString("") { it.text } }

@OptIn(ExperimentalTestApi::class)
class NativePaneActionsTest {
    @Test
    fun theListMenuOffersTheSessionsOwnVerbsInPlaceOfHerdrsClose() = runComposeUiTest {
        val cases = listOf(
            Triple(SESSION, true, listOf("Open", "Rename…", "Show on desk", "Close")),
            Triple(SESSION, false, listOf("Open", "Rename…", "Close")),
            Triple(ENDED, true, listOf("Open", "Rename…", "Forget")),
        )
        var current by mutableStateOf(cases.first())
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(900.dp, 600.dp)) {
                    PaneMenu(Breakpoint.Desktop, current.first, MenuAnchor(120.dp, 120.dp), null, {}, {}, {}, desk = current.second)
                }
            }
        }
        for (case in cases) {
            current = case
            waitForIdle()
            assertEquals(case.third, texts(), "${case.first.exited} desk=${case.second}")
        }
    }

    @Test
    fun showingOnTheDeskIsOnePressAndClosingAsksFirst() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        menu(SESSION, true, sent)
        onNodeWithText("Show on desk").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.NativeDesk(SESSION.id)), sent)

        onNodeWithText("Close").performClick()
        waitForIdle()
        assertEquals(1, sent.size, "the session was closed without asking: $sent")
        onNodeWithText("close").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.NativeDesk(SESSION.id), ManageOp.NativeClose(SESSION.id)), sent)
    }

    @Test
    fun anEndedSessionIsForgottenOnlyOnceConfirmed() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        menu(ENDED, true, sent)
        onNodeWithText("Forget").performClick()
        waitForIdle()
        assertTrue(sent.isEmpty(), "forgot without asking: $sent")
        onNodeWithText("forget").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.NativeForget(ENDED.id)), sent)
    }

    @Test
    fun theInSessionSheetDropsHerdrsDeskControlsAndClosesTheSession() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        sheet(SESSION, true, sent)
        waitForIdle()
        val shown = texts()
        for (absent in listOf("focus at the desk", "fill the tab", "remember the splits", "tab", "workspace")) {
            assertTrue(absent !in shown, "a kept session's sheet offers herdr's \"$absent\": $shown")
        }
        assertTrue("not on desk" in shown.joinToString("\n"), "the sheet does not say the session is off the desk: $shown")
        onNodeWithText("show on desk").performClick()
        waitForIdle()
        onNodeWithText("close").performClick()
        waitForIdle()
        onAllNodesWithText("close")[1].performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.NativeDesk(SESSION.id), ManageOp.NativeClose(SESSION.id)), sent)
    }

    @Test
    fun theInSessionSheetForgetsAnEndedOneAndSaysHowItEnded() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        sheet(ENDED, true, sent)
        waitForIdle()
        onNodeWithText("killed by signal 9").assertExists()
        onNodeWithText("show on desk").assertDoesNotExist()
        onNodeWithText("forget").performClick()
        waitForIdle()
        onAllNodesWithText("forget")[1].performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.NativeForget(ENDED.id)), sent)
    }

    @Test
    fun aSidebarRowMarksASessionAndSaysSo() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(296.dp, 80.dp)) { PaneRow(ENDED, 0.0, false) {} }
            }
        }
        waitForIdle()
        onNodeWithText("kampr · killed by signal 9").assertExists()
        onNodeWithContentDescription("Kampr session, killed by signal 9", substring = true).assertExists()
    }
}
