package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.MenuAnchor
import dev.kampr.shared.ui.PaneActionsSheet
import dev.kampr.shared.ui.PaneMenu
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val LISTED = PaneInfo(
    id = "01JNODE/w3:p2",
    nodeId = "01JNODE",
    workspaceId = "01JNODE/w3",
    tabId = "01JNODE/w3:t1",
    workspace = "kampr",
    tab = "1",
    label = "claude",
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = "done",
    cols = 74,
    rows = 30,
)

private val SIBLING = LISTED.copy(id = "01JNODE/w3:p3", label = "tests")

// Everything the in-session sheet says about the machine's own screen. None of it may appear on a
// surface where the pane is a row: the operator cannot see the desk it reshapes, and half of it
// (#265's zoom, #396's focus) changes what somebody else is looking at.
private val DESK = listOf("focus at the desk", "fill the tab", "remember the splits", "put the splits back")

@OptIn(ExperimentalTestApi::class)
@Composable
private fun Menu(
    sent: MutableList<ManageOp>,
    breakpoint: Breakpoint = Breakpoint.Portrait,
    anchor: MenuAnchor? = null,
    opened: MutableList<String> = mutableListOf(),
) {
    CompositionLocalProvider(LocalTokens provides phoneTokens()) {
        Box(Modifier.size(900.dp, 600.dp)) {
            PaneMenu(breakpoint, LISTED, anchor, null, { sent += it }, { opened += LISTED.id }, {})
        }
    }
}

@OptIn(ExperimentalTestApi::class)
@Composable
private fun InSession(sent: MutableList<ManageOp>) {
    CompositionLocalProvider(LocalTokens provides phoneTokens()) {
        Box(Modifier.fillMaxSize()) {
            PaneActionsSheet(
                Breakpoint.Portrait, LISTED, null, { sent += it }, {},
                panes = listOf(LISTED, SIBLING),
            )
        }
    }
}

// The operator, verbatim: "i guess the sidebar menu should probably only be relevant to things
// that actually affect the sidebar, as these sessions might not be visible at the moment". Probe
// #426 measured herdr answering the same question the same way — two items on a sidebar row, and
// a pane's own six only where a pane actually is.
@OptIn(ExperimentalTestApi::class)
class PaneMenuTest {
    // Flat, and exactly this long. The anchored box carries no header, so every word in it is an
    // item — which makes the whole menu assertable rather than only the three that should be there.
    @Test
    fun theListMenuIsThreeVerbsAndNothingElse() = runComposeUiTest {
        setContent { Menu(mutableListOf(), Breakpoint.Desktop, MenuAnchor(120.dp, 120.dp)) }
        waitForIdle()
        assertEquals(
            listOf("Open", "Rename…", "Close"),
            onAllNodesWithText("", substring = true)
                .fetchSemanticsNodes()
                .mapNotNull { node ->
                    node.config.getOrElseNullable(SemanticsProperties.Text) { null }
                        ?.joinToString("") { line -> line.text }
                },
            "the list menu is not the three verbs and nothing else",
        )
    }

    @Test
    fun theListMenuCarriesNothingAboutTheDesk() = runComposeUiTest {
        setContent { Menu(mutableListOf()) }
        waitForIdle()
        for (control in DESK) {
            val written = onAllNodesWithText(control, substring = true).fetchSemanticsNodes().size
            val spoken = onAllNodesWithContentDescription(control, substring = true).fetchSemanticsNodes().size
            assertEquals(
                0,
                written + spoken,
                "the list menu offers \"$control\", which is about a screen the operator cannot see",
            )
        }
    }

    // Rule 3, and #426's own measurement of it: herdr's tab strip stayed on the focused workspace
    // for the whole time an unfocused one's menu was open. A menu that focused, selected or
    // resized the row it was opened on would destroy the operator's unread marker on open (#396).
    @Test
    fun openingTheListMenuPutsNothingOnTheWire() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        val opened = mutableListOf<String>()
        setContent { Menu(sent, opened = opened) }
        waitForIdle()
        assertTrue(sent.isEmpty(), "opening the list menu sent $sent")
        assertTrue(opened.isEmpty(), "opening the list menu opened the pane under it")
    }

    @Test
    fun openingTheInSessionSheetPutsNothingOnTheWire() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent { InSession(sent) }
        waitForIdle()
        assertTrue(sent.isEmpty(), "opening the actions sheet sent $sent")
    }

    // Herdr names the object and counts its panes before it will act — `Close workspace?` over
    // `bravo — 1 pane` (#426). The sheet used to say "Closes every pane in it." and count nothing.
    @Test
    fun closingATabNamesItAndCountsThePanesItWillTakeWithIt() = runComposeUiTest {
        setContent { InSession(mutableListOf()) }
        waitForIdle()
        onAllNodesWithText("close")[1].performClick()
        waitForIdle()
        onNodeWithText("Close tab?").assertExists("the tab's confirmation does not name the kind")
        onNodeWithText("1 — 2 panes").assertExists("the tab's confirmation does not count its panes")
    }

    @Test
    fun closingAPaneFromTheListMenuNamesItFirst() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent { Menu(sent) }
        waitForIdle()
        onNodeWithText("Close").performClick()
        waitForIdle()
        assertTrue(sent.isEmpty(), "the list menu closed the pane without asking: $sent")
        assertEquals(
            listOf("Close pane? ${paneTitle(LISTED)}. Confirm or cancel."),
            onAllNodesWithContentDescription("Close pane?", substring = true)
                .fetchSemanticsNodes()
                .map { it.config[SemanticsProperties.ContentDescription].first() },
            "the list menu's confirmation does not name the kind and the pane the way herdr's does",
        )
        onNodeWithText("close").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.Close(LISTED.id)), sent, "the confirmed close sent $sent")
    }

    @Test
    fun theInSessionSheetStillCarriesEveryDeskControl() = runComposeUiTest {
        setContent { InSession(mutableListOf()) }
        waitForIdle()
        for (control in listOf("focus at the desk", "fill the tab", "remember the splits")) {
            assertTrue(
                onAllNodesWithText(control).fetchSemanticsNodes().isNotEmpty(),
                "the actions sheet lost \"$control\"",
            )
        }
    }

    // The two chips sent the identical `pane.zoom`, so the sheet offered one control twice under
    // two names and `ManageOp.Focus` was constructed nowhere in the client.
    @Test
    fun theFocusChipAndTheFillChipAreNoLongerTheSameOp() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent { InSession(sent) }
        waitForIdle()
        onAllNodesWithText("focus at the desk").onFirst().performClick()
        waitForIdle()
        onNodeWithText("focus").performClick()
        waitForIdle()
        onNodeWithText("fill the tab").performClick()
        waitForIdle()
        assertEquals(
            listOf<ManageOp>(ManageOp.Focus(LISTED.id), ManageOp.PaneZoom(LISTED.id)),
            sent,
            "the two chips sent $sent",
        )
    }

    // Focus is the one op that destroys herdr's `done`, the operator's unread flag (#357, #396),
    // and somebody pressing it to find a pane must not clear their own marker unasked.
    @Test
    fun theFocusChipSaysWhatItCostsBeforeItSendsAnything() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent { InSession(sent) }
        waitForIdle()
        onAllNodesWithText("focus at the desk").onFirst().performClick()
        waitForIdle()
        assertTrue(sent.isEmpty(), "the focus chip sent $sent before it said what it costs")
        assertTrue(
            onAllNodesWithText("done marker", substring = true).fetchSemanticsNodes().isNotEmpty(),
            "the focus chip never mentions the done marker it destroys",
        )
    }

    // Herdr anchors a context menu at the pointer's own cell (#426). A phone has no pointer and
    // gets the bottom sheet; a desk that ignored the anchor would open every menu in one corner.
    @Test
    fun aDesktopMenuHangsOffThePlaceItWasOpenedFrom() = runComposeUiTest {
        setContent { Menu(mutableListOf(), Breakpoint.Desktop, MenuAnchor(400.dp, 220.dp)) }
        waitForIdle()
        val at = onNodeWithText("Open").fetchSemanticsNode().positionInRoot
        val x = with(density) { at.x.toDp() }
        val y = with(density) { at.y.toDp() }
        assertTrue(x > 380.dp && x < 460.dp, "the menu opened at x=$x rather than beside its 400 dp anchor")
        assertTrue(y > 200.dp && y < 290.dp, "the menu opened at y=$y rather than beside its 220 dp anchor")
    }
}
