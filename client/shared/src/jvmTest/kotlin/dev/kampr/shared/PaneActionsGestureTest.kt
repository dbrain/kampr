package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.rightClick
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.MenuAnchor
import dev.kampr.shared.ui.PaneCard
import dev.kampr.shared.ui.PaneRow
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
    cwd = "/home/dbrain/dev/kampr",
    agent = "claude",
    agentStatus = "working",
    cols = 74,
    rows = 30,
)

private const val NOW = 1_700_000_000.0

private const val ELLIPSIS = "Pane menu"

// Two menus, and which one a listed pane reaches is the whole point: a row is a pane you cannot
// see, so it gets the list menu and never the in-session sheet.
private class ManageSpy(override val enabled: Boolean = true) : ManageIo {
    val opened = mutableListOf<String>()
    val sheeted = mutableListOf<String>()
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) {
        sheeted += paneId
    }

    override fun openMenu(paneId: String, at: MenuAnchor?) {
        opened += paneId
        anchors += at
    }

    val anchors = mutableListOf<MenuAnchor?>()
}

private enum class Listing { Card, Row }

private data class Place(val listing: Listing, val selecting: Boolean, val said: String)

// The sidebar sits under a SelectionContainer on the desk, and a container that eats the press is
// exactly the shape of defect that hid this one.
private val PLACES = listOf(
    Place(Listing.Card, false, "a herd-list card"),
    Place(Listing.Card, true, "a herd-list card inside a SelectionContainer"),
    Place(Listing.Row, false, "a sidebar row"),
    Place(Listing.Row, true, "a sidebar row inside a SelectionContainer"),
)

@Composable
private fun Listed(manage: ManageIo, place: Place, onClick: () -> Unit) {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalManage provides manage) {
        val body: @Composable () -> Unit = {
            Box(Modifier.size(320.dp, 120.dp)) {
                when (place.listing) {
                    Listing.Card -> PaneCard(LISTED, NOW, onClick)
                    Listing.Row -> PaneRow(LISTED, NOW, active = false, onClick = onClick)
                }
            }
        }
        if (place.selecting) SelectionContainer { body() } else body()
    }
}

private data class Outcome(val opened: List<String>, val sheeted: List<String>, val clicked: Int)

@OptIn(ExperimentalTestApi::class)
private fun gesture(place: Place, on: SemanticsNodeInteraction.() -> Unit): Outcome {
    val manage = ManageSpy()
    var clicked = 0
    lateinit var outcome: Outcome
    runComposeUiTest {
        setContent { Listed(manage, place) { clicked++ } }
        waitForIdle()
        onNodeWithContentDescription("Open ", substring = true).on()
        waitForIdle()
        outcome = Outcome(manage.opened.toList(), manage.sheeted.toList(), clicked)
    }
    return outcome
}

// Both gestures shipped dead on every platform. `awaitFirstDown` never returns for a
// secondary-button press on skiko — Compose asks it for the primary button and nothing else — and
// the `clickable` inside `Modifier.action` consumed the held press out from under the long press,
// so the one surface that never had a "…" had no way in at all.
@OptIn(ExperimentalTestApi::class)
class PaneActionsGestureTest {
    @Test
    fun aRightClickOpensThePanesActionsWhereverAPaneIsListed() {
        for (place in PLACES) {
            val outcome = gesture(place) { performMouseInput { rightClick() } }
            assertEquals(
                listOf(LISTED.id),
                outcome.opened,
                "a right-click on ${place.said} opened ${outcome.opened}",
            )
        }
    }

    @Test
    fun aLongPressOpensThePanesActionsWhereverAPaneIsListed() {
        for (place in PLACES) {
            val outcome = gesture(place) { performTouchInput { longClick() } }
            assertEquals(
                listOf(LISTED.id),
                outcome.opened,
                "a long press on ${place.said} opened ${outcome.opened}",
            )
        }
    }

    // A gesture that also opens the pane it was aimed at has answered a different question: the
    // sheet would come up behind the screen the click navigated to.
    @Test
    fun neitherGestureAlsoOpensThePaneUnderIt() {
        for (place in PLACES) {
            val right = gesture(place) { performMouseInput { rightClick() } }
            assertEquals(0, right.clicked, "a right-click on ${place.said} also opened the pane")
            val long = gesture(place) { performTouchInput { longClick() } }
            assertEquals(0, long.clicked, "a long press on ${place.said} also opened the pane")
        }
    }

    @Test
    fun anOrdinaryTapStillOpensThePane() {
        for (place in PLACES) {
            val outcome = gesture(place) { performClick() }
            assertEquals(1, outcome.clicked, "a tap on ${place.said} did not open the pane")
            assertTrue(
                outcome.opened.isEmpty(),
                "a tap on ${place.said} opened the actions sheet: ${outcome.opened}",
            )
        }
    }

    // The gesture is a shortcut, never the only way in — which was a promise the herd list and the
    // sidebar did not keep, on the two surfaces where a finger has nothing else to press.
    @Test
    fun everySurfaceThatCarriesTheGestureAlsoCarriesTheEllipsis() {
        for (place in PLACES) {
            val manage = ManageSpy()
            runComposeUiTest {
                setContent { Listed(manage, place) {} }
                waitForIdle()
                onNodeWithContentDescription(ELLIPSIS).performClick()
                waitForIdle()
                assertEquals(
                    listOf(LISTED.id),
                    manage.opened,
                    "the ellipsis on ${place.said} opened ${manage.opened}",
                )
            }
        }
    }

    // The list menu is the sidebar's, and the in-session sheet is the pane screen's and the mosaic
    // cell's. A row that opened the sheet would be offering "fill the tab" about a tab nobody is
    // looking at, which is the thing the rework is for.
    @Test
    fun aListedPaneReachesTheListMenuAndNeverTheInSessionSheet() {
        for (place in PLACES) {
            val right = gesture(place) { performMouseInput { rightClick() } }
            assertTrue(
                right.sheeted.isEmpty(),
                "a right-click on ${place.said} opened the in-session sheet: ${right.sheeted}",
            )
            val manage = ManageSpy()
            runComposeUiTest {
                setContent { Listed(manage, place) {} }
                waitForIdle()
                onNodeWithContentDescription(ELLIPSIS).performClick()
                waitForIdle()
                assertTrue(
                    manage.sheeted.isEmpty(),
                    "the ellipsis on ${place.said} opened the in-session sheet: ${manage.sheeted}",
                )
            }
        }
    }

    // Herdr anchors a context menu at the pointer's own cell (#426). A menu that always opened at
    // the top-left would still be a menu, and would still be wrong on a 27-inch screen.
    @Test
    fun aRightClickHandsTheMenuThePointersOwnPlace() {
        for (place in PLACES) {
            val manage = ManageSpy()
            runComposeUiTest {
                setContent { Listed(manage, place) {} }
                waitForIdle()
                onNodeWithContentDescription("Open ", substring = true)
                    .performMouseInput { rightClick(percentOffset(0.75f, 0.5f)) }
                waitForIdle()
                val at = manage.anchors.singleOrNull()
                assertTrue(
                    at != null && at.x > 100.dp,
                    "a right-click three quarters across a 320 dp ${place.said} anchored at $at",
                )
            }
        }
    }

    // `caps.manage` false means absent rather than present-and-failing, and a right-click that
    // silently swallows itself on a read-only device is the second of those.
    @Test
    fun aReadOnlyDeviceGetsNeitherTheGestureNorTheEllipsis() {
        for (place in PLACES) {
            val manage = ManageSpy(enabled = false)
            var clicked = 0
            runComposeUiTest {
                setContent { Listed(manage, place) { clicked++ } }
                waitForIdle()
                assertEquals(
                    0,
                    onAllNodesWithContentDescription(ELLIPSIS).fetchSemanticsNodes().size,
                    "a read-only ${place.said} still painted the ellipsis",
                )
                onNodeWithContentDescription("Open ", substring = true)
                    .performMouseInput { rightClick() }
                onNodeWithContentDescription("Open ", substring = true)
                    .performTouchInput { longClick() }
                waitForIdle()
                assertTrue(
                    manage.opened.isEmpty(),
                    "a read-only ${place.said} opened ${manage.opened}",
                )
            }
        }
    }

    // TalkBack never sends a pointer event, so the long press it offers is a semantic action or it
    // is nothing. Swapping the pointer handling underneath must not take that away.
    @Test
    fun aScreenReaderStillGetsTheLongPressAsAnActionItCanDispatch() {
        for (place in PLACES) {
            val manage = ManageSpy()
            runComposeUiTest {
                setContent { Listed(manage, place) {} }
                waitForIdle()
                val action = onNodeWithContentDescription("Open ", substring = true)
                    .fetchSemanticsNode()
                    .config
                    .getOrElseNullable(SemanticsActions.OnLongClick) { null }
                assertTrue(action != null, "${place.said} offers no OnLongClick to a screen reader")
                action.action?.invoke()
                waitForIdle()
                assertEquals(
                    listOf(LISTED.id),
                    manage.opened,
                    "the semantic long press on ${place.said} opened ${manage.opened}",
                )
            }
        }
    }
}
