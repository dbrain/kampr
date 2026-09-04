package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.KeyboardFloor
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.PhoneScaffold
import dev.kampr.shared.ui.Screen
import kotlin.test.Test
import kotlin.test.assertEquals

// A pane is the one screen that does not pay the status bar through `screenInset` — its header
// pays it inside its own background instead, so the grid underneath is never letterboxed. Reaching
// a pane *from* a screen that does pay it left the pane's whole subtree reporting a position one
// status bar below the place it draws, and everything that asks a layout where it is believed the
// report: every selection handle, every text toolbar, every popup. Measured on a Pixel 10 Pro at
// 172 px — the handles sat under the Copy toolbar, a line and a half below their own words.
//
// The mechanism is the modifier chain changing *shape*: `screenInset` returned the receiver
// untouched for a pane and an `absolutePadding` for everything else, so the node kept a stale
// position across the switch. Padding a pane with zero keeps one shape and the staleness has
// nowhere to live.
//
// `positionInWindow` is what a `Popup` anchors to; `boundsInWindow` is where the pane paints. The
// two disagreeing is the whole defect, which is why this needs no rendered handle to see — and it
// is why the suite was blind to it before: desktop draws no selection handle at all.
@OptIn(ExperimentalTestApi::class)
class PaneInsetCoordinatesTest {
    @Test
    fun a_pane_reached_from_the_herd_reports_the_place_it_is_drawn() = runComposeUiTest {
        var screen: Screen by mutableStateOf(Screen.Herd)
        var reported = Offset.Unspecified
        var drawn = Rect.Zero
        setContent {
            Bars {
                val edge = LocalSafeArea.current
                KeyboardFloor(Modifier.fillMaxSize()) {
                    PhoneScaffold(Breakpoint.Portrait, screen, edge, {}) {
                        Box(
                            Modifier.fillMaxSize().onGloballyPositioned {
                                reported = it.positionInWindow()
                                drawn = it.boundsInWindow()
                            },
                        )
                    }
                }
            }
        }
        waitForIdle()

        // Twice around, because a screen is somewhere you come back to: the defect was carried in
        // by the *switch*, so a pane visited once proves less than a pane returned to.
        for (visit in 1..2) {
            for (away in listOf(Screen.Herd, Screen.Setup)) {
                screen = away
                waitForIdle()
                screen = Screen.Pane("01JNODE.../w3:p2", PaneView.Conversation)
                waitForIdle()

                assertEquals(
                    drawn.top,
                    reported.y,
                    "visit $visit by way of $away: the pane reports itself " +
                        "${reported.y - drawn.top} px from where it draws, so every handle and " +
                        "toolbar anchored to it lands there too",
                )
            }
        }
    }
}
