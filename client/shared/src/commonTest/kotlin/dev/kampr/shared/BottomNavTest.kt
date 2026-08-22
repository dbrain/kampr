package dev.kampr.shared

import androidx.compose.ui.unit.dp
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.SafeArea
import dev.kampr.shared.ui.barCovered
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.Tab
import dev.kampr.shared.ui.bottomChrome
import dev.kampr.shared.ui.screenFor
import dev.kampr.shared.ui.tabFor
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// A pixel_6: a status bar with a punch-hole in it, and a gesture handle.
private val PHONE = SafeArea(top = 44.dp, bottom = 46.dp)

private val EVERY_SCREEN = listOf(
    Screen.Herd,
    Screen.Mosaic,
    Screen.Pane("x", PaneView.Terminal),
    Screen.Setup,
    Screen.Devices,
    Screen.Appearance,
    Screen.Notifications,
)

class BottomNavTest {
    // A tab that leads somewhere it does not then light up on is the "Pane" defect exactly: it led
    // to the pane last opened and stayed lit after the herd had been gone back to. Both directions,
    // because a mapping that answered Herd to everything would pass one of them.
    @Test
    fun everyTabLeadsSomewhereThatLightsItBackUp() {
        for (tab in Tab.entries) assertEquals(tab, tabFor(screenFor(tab)), "$tab")
    }

    // Nothing in the app is outside the two stacks, or the bar shows a screen with no tab selected
    // and the reader has no idea where they are.
    @Test
    fun everyScreenSitsUnderATab() {
        assertEquals(
            listOf(Tab.Herd, Tab.Herd, Tab.Herd, Tab.Settings, Tab.Settings, Tab.Settings, Tab.Settings),
            EVERY_SCREEN.map(::tabFor),
        )
    }

    // Phone landscape has its own layout and had no navigation at all for a while: Setup, Devices,
    // Appearance and Notifications were reachable from nowhere. The tabs are what leads there, so
    // the posture that draws them is the posture that can reach them.
    @Test
    fun settingsIsReachableFromTheHerdInEveryPostureWithTabs() {
        for (breakpoint in listOf(Breakpoint.Portrait, Breakpoint.Landscape)) {
            assertTrue(bottomChrome(breakpoint, Screen.Herd), "$breakpoint draws no tab bar on the herd")
        }
        assertEquals(Screen.Setup, screenFor(Tab.Settings))
    }

    // The keyboard takes the pane's tab bar by *height*, not by switching it off: a boolean keyed
    // on the endpoint of a 250 ms animation has no partially-uncovered state, so the bar arrived
    // whole in the frame after the keys had gone. The subtraction is the gesture handle, which the
    // keys are over before they are over anything of the bar's own.
    @Test
    fun theKeysTakeThePanesTabBarByDegreesAndNotAtAStroke() {
        for (view in PaneView.entries) {
            val pane = Screen.Pane("x", view)
            assertTrue(bottomChrome(Breakpoint.Portrait, pane), "$view loses its bar outright")
            assertEquals(0.dp, barCovered(Breakpoint.Portrait, pane, PHONE), "$view with the keyboard down")
            assertEquals(
                0.dp,
                barCovered(Breakpoint.Portrait, pane, PHONE.copy(ime = 30.dp)),
                "$view: the keys are only over the handle the bar had already stopped paying for",
            )
            assertEquals(
                254.dp,
                barCovered(Breakpoint.Portrait, pane, PHONE.copy(ime = 300.dp)),
                "$view with the keyboard up",
            )
        }
    }

    // Everywhere else the keyboard is over a scrolling form, and losing the tabs would strand a
    // reader who opened one by accident.
    @Test
    fun everyOtherScreenKeepsItsTabs() {
        for (screen in listOf(Screen.Herd, Screen.Setup, Screen.Devices, Screen.Appearance, Screen.Notifications)) {
            assertTrue(bottomChrome(Breakpoint.Portrait, screen), "$screen has no tab bar")
            for (ime in listOf(0.dp, 300.dp)) {
                assertEquals(
                    0.dp,
                    barCovered(Breakpoint.Portrait, screen, PHONE.copy(ime = ime)),
                    "$screen loses its tabs to a keyboard at ime=$ime",
                )
            }
        }
    }

    // The third case, and the one a rule written for a portrait phone gets wrong: rotated, a pane
    // wears no tab bar at all, so its key row is the last thing in the window with the keyboard
    // down as well as up and owes the gesture handle either way.
    @Test
    fun aRotatedPaneHasNothingUnderItWhicheverWayTheKeyboardIs() {
        val pane = Screen.Pane("x", PaneView.Terminal)
        assertFalse(bottomChrome(Breakpoint.Landscape, pane), "a rotated pane wears a tab bar")
        assertTrue(bottomChrome(Breakpoint.Landscape, Screen.Herd), "a rotated herd does not")
        for (ime in listOf(0.dp, 300.dp)) {
            assertEquals(
                0.dp,
                barCovered(Breakpoint.Landscape, pane, PHONE.copy(ime = ime)),
                "ime=$ime: rotated, there is no bar for the keys to uncover",
            )
        }
    }

    // The desktop ends in its status strip on every screen there is, keyboard or no keyboard.
    @Test
    fun theDesktopAlwaysEndsInItsStatusStrip() {
        for (screen in listOf(Screen.Pane("x", PaneView.Split), Screen.Herd, Screen.Setup)) {
            assertTrue(bottomChrome(Breakpoint.Desktop, screen), "$screen")
            assertEquals(0.dp, barCovered(Breakpoint.Desktop, screen, PHONE.copy(ime = 300.dp)), "$screen")
        }
    }
}
