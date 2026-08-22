package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsNotDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.KeyboardFloor
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PhoneScaffold
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.named
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertTrue

private const val BODY = "Screen body"
private val PANE = Screen.Pane("01JNODE/w1:p1", PaneView.Terminal)
private val TABS = listOf("Herd tab", "Settings tab")

// Gboard's travel, sampled the way the system moves it: a value, not a switch. The last few
// entries are the ones that matter — that is where a bar keyed on `ime == 0` has nothing to say.
private val TRAVEL = listOf(0.dp, 6.dp, 16.dp, 30.dp, 46.dp, 60.dp, 90.dp, 140.dp, 220.dp, 300.dp)

// One step of the keyboard, and where the app had put itself once it had landed.
private class Step(val ime: Dp, val floor: Dp, val owed: Dp, val tabs: List<Dp>, val window: Dp) {
    // How much of the bar is out from under the keys. Its ground runs to the bottom of the window,
    // so what the screen above it does not take, and the keys have not covered, is the bar.
    val exposed: Dp get() = window - ime - floor
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.tabTops(): List<Dp> = TABS
    .flatMap { onAllNodesWithContentDescription(it).fetchSemanticsNodes() }
    .map { with(density) { it.boundsInRoot.top.toDp() } }

// The window as the app itself stacks it: `KeyboardFloor` and `PhoneScaffold` are the app's own
// pieces, wired here exactly as `AppScaffold` wires them, so a fix that only reached the test
// could not make this pass.
//
// The bars are state the sweep moves *after* the first frame has settled, because the defect is a
// value that moves — a window composed with the keyboard already up, or already down, is a
// vacuous test of a keyboard that is on its way somewhere.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.sweep(screen: Screen, travel: List<Dp>): List<Step> {
    var bars by mutableStateOf(BARS)
    var owed = Dp.Unspecified
    setContent {
        Bars(bars) {
            val edge = LocalSafeArea.current
            KeyboardFloor(Modifier.fillMaxSize()) {
                PhoneScaffold(Breakpoint.Portrait, screen, edge, {}) {
                    owed = LocalSafeArea.current.bottom
                    Box(Modifier.fillMaxSize().named(BODY))
                }
            }
        }
    }
    waitForIdle()
    val window = onRoot().getUnclippedBoundsInRoot().bottom
    return travel.map { ime ->
        bars = BARS.copy(ime = ime)
        waitForIdle()
        Step(ime, onNodeWithContentDescription(BODY).getUnclippedBoundsInRoot().bottom, owed, tabTops(), window)
    }
}

// The report, verbatim: "when you minimise keyboard the bottom bar jumps into vision instead of
// being there already".
//
// `WindowInsets.ime` is animated from the keyboard's height down to zero over roughly 250 ms, and
// a bar switched on that value's *endpoint* has no partially-revealed state — so the key row
// slides down with the keys and then the tab bar appears whole, at the bottom, in one frame.
@OptIn(ExperimentalTestApi::class)
class BottomBarRevealTest {
    // Nothing may move further than the keys moved. One frame of a 250 ms animation is a few dp of
    // travel; the bar arriving inside one of them is a hundred.
    @Test
    fun theBarIsUncoveredByTheKeysRatherThanArrivingOnceTheyHaveGone() = runComposeUiTest {
        val steps = sweep(PANE, TRAVEL + TRAVEL.reversed())
        for ((before, after) in steps.zipWithNext()) {
            val moved = abs((after.floor - before.floor).value)
            val keys = abs((after.ime - before.ime).value)
            assertTrue(
                moved <= keys + 1f,
                "the keys moved $keys dp between ime=${before.ime} and ime=${after.ime}, and the " +
                    "screen above the bar moved $moved dp — from ${before.floor} to ${after.floor}",
            )
        }
    }

    // And it has to be uncovered by *degrees*, or the assertion above is satisfied by a bar that
    // is never drawn at all. Three sampled positions strictly between hidden and whole, which is
    // the state a boolean keyed on the endpoint cannot represent.
    @Test
    fun theBarIsPartlyOutFromUnderTheKeysWhileTheyAreStillLeaving() = runComposeUiTest {
        val steps = sweep(PANE, TRAVEL)
        val whole = steps.first { it.ime == 0.dp }.exposed
        assertTrue(whole > 40.dp, "the bar is only $whole tall with the keyboard down, so it was never drawn")
        val partly = steps.filter { it.ime > 0.dp && it.exposed > 0.dp && it.exposed < whole }
        assertTrue(
            partly.size >= 3,
            "the bar was partly uncovered at ${partly.size} of ${steps.size} sampled keyboard " +
                "heights — ${steps.map { "${it.ime}=${it.exposed}" }}",
        )
    }

    // Uncovered, not slid in: the tabs are where they will end up from the first frame they show
    // in, and nothing opens up between them and the screen above.
    @Test
    fun theBarDoesNotMoveWhileItIsBeingUncovered() = runComposeUiTest {
        val steps = sweep(PANE, TRAVEL)
        val resting = steps.first { it.ime == 0.dp }
        val showing = steps.filter { it.exposed > 0.dp }
        assertTrue(showing.size >= 4, "the bar was only ever visible at ${showing.size} of ${steps.size} heights")
        for (step in showing) {
            assertTrue(step.tabs.isNotEmpty(), "no tabs at ime=${step.ime}, so nothing was measured")
            assertTrue(
                step.tabs.zip(resting.tabs).all { (moving, still) -> abs((moving - still).value) < 1f },
                "at ime=${step.ime} the tabs are at ${step.tabs}, and at rest they are at ${resting.tabs}",
            )
        }
    }

    // The keys are over the bar, not over the pane: nothing of a covered bar may be reachable, or
    // a reader is offered a control the eye cannot see.
    @Test
    fun aBarTheKeysHaveCoveredIsNotOnScreen() = runComposeUiTest {
        val steps = sweep(PANE, listOf(0.dp, 300.dp))
        assertTrue(steps.last().exposed <= 0.dp, "the bar still shows ${steps.last().exposed} under a 300 dp keyboard")
        for (tab in TABS) onNodeWithContentDescription(tab).assertIsNotDisplayed()
    }

    // The bar under the screen is what owes the gesture handle, and the rule that settled the
    // 46 dp dead strip is that only one thing may owe it. A pane whose bar came and went was told
    // it owed the handle again for the last frames of every dismissal — which is the same strip,
    // opening exactly where the bar was about to be uncovered.
    @Test
    fun nothingAboveTheBarEverOwesTheHandleAsWell() = runComposeUiTest {
        for (step in sweep(PANE, TRAVEL)) {
            assertTrue(
                step.owed == 0.dp,
                "at ime=${step.ime} the pane was told it owes ${step.owed} at its bottom edge, and " +
                    "the bar under it is paying for the same handle",
            )
        }
    }

    // Everywhere else the keyboard is over a scrolling form, and losing the tabs would strand a
    // reader who opened one by accident. Only the pane's bar is the keyboard's to take.
    @Test
    fun everyOtherScreenKeepsItsTabsThroughout() = runComposeUiTest {
        for (step in sweep(Screen.Herd, TRAVEL)) {
            assertTrue(step.exposed > 40.dp, "the herd's tab bar was ${step.exposed} tall at ime=${step.ime}")
        }
    }
}
