package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
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
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val BODY = "Screen body"
private val PANE = Screen.Pane("01JNODE/w1:p1", PaneView.Terminal)
private val TABS = listOf("Herd tab", "Settings tab")

private val TRAVEL = listOf(0.dp, 6.dp, 16.dp, 30.dp, 46.dp, 60.dp, 90.dp, 140.dp, 220.dp, 300.dp)

private class Step(val ime: Dp, val floor: Dp, val owed: Dp, val tabs: Int, val window: Dp) {
    val under: Dp get() = window - ime - floor
}

// The window as the app itself stacks it: `KeyboardFloor` and `PhoneScaffold` are the app's own
// pieces, wired here exactly as `AppScaffold` wires them. The keyboard moves after the first frame
// has settled, because it is a value on its way somewhere and not a switch.
@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.sweep(screen: Screen): List<Step> {
    var bars by mutableStateOf(BARS)
    var owed = Dp.Unspecified
    setContent {
        Bars(bars) {
            KeyboardFloor(Modifier.fillMaxSize()) {
                PhoneScaffold(Breakpoint.Portrait, screen, {}) {
                    owed = LocalSafeArea.current.bottom
                    Box(Modifier.fillMaxSize().named(BODY))
                }
            }
        }
    }
    waitForIdle()
    val window = onRoot().getUnclippedBoundsInRoot().bottom
    return (TRAVEL + TRAVEL.reversed()).map { ime ->
        bars = BARS.copy(ime = ime)
        waitForIdle()
        val tabs = TABS.sumOf { onAllNodesWithContentDescription(it).fetchSemanticsNodes().size }
        Step(ime, onNodeWithContentDescription(BODY).getUnclippedBoundsInRoot().bottom, owed, tabs, window)
    }
}

// Only one thing may owe the gesture handle: whatever ends at the bottom of the window. On a pane
// that is the pane, all the way through a keyboard's travel; on every other screen it is the tabs.
@OptIn(ExperimentalTestApi::class)
class PhoneBottomEdgeTest {
    @Test
    fun aPaneReachesTheKeysAndOwesTheHandleItselfWhereverTheKeyboardIs() = runComposeUiTest {
        for (step in sweep(PANE)) {
            assertEquals(0, step.tabs, "a tab bar under a pane at ime=${step.ime}")
            assertTrue(step.under <= 0.5.dp, "${step.under} of something under the pane at ime=${step.ime}")
            assertEquals(
                (BARS.bottom - step.ime).coerceAtLeast(0.dp),
                step.owed,
                "at ime=${step.ime} the pane was told it owes ${step.owed} at its bottom edge",
            )
        }
    }

    @Test
    fun everyOtherScreenKeepsItsTabsThroughoutAndOwesNothingUnderThem() = runComposeUiTest {
        for (step in sweep(Screen.Herd)) {
            assertEquals(TABS.size, step.tabs, "the herd's tabs at ime=${step.ime}")
            assertTrue(step.under > 40.dp, "the herd's tab bar was ${step.under} tall at ime=${step.ime}")
            assertEquals(0.dp, step.owed, "the herd owes the handle over its own tab bar at ime=${step.ime}")
        }
    }
}
