package dev.kampr.shared

import androidx.compose.ui.unit.dp
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.bottomChrome
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class BottomNavTest {
    @Test
    fun theTabBarStandsDownWhileAPaneIsBeingTypedInto() {
        for (view in PaneView.entries) {
            assertFalse(bottomChrome(Breakpoint.Portrait, Screen.Pane("x", view), 300.dp), "$view with the keyboard up")
            assertTrue(bottomChrome(Breakpoint.Portrait, Screen.Pane("x", view), 0.dp), "$view with the keyboard down")
        }
    }

    // Everywhere else the keyboard is over a scrolling form, and losing the tabs would strand a
    // reader who opened one by accident.
    @Test
    fun everyOtherScreenKeepsItsTabs() {
        for (screen in listOf(Screen.Herd, Screen.Setup, Screen.Devices, Screen.Appearance, Screen.Notifications)) {
            assertTrue(bottomChrome(Breakpoint.Portrait, screen, 300.dp), "$screen with the keyboard up")
            assertTrue(bottomChrome(Breakpoint.Portrait, screen, 0.dp), "$screen with the keyboard down")
        }
    }

    // The third case, and the one a rule written for a portrait phone gets wrong: rotated, a pane
    // wears no tab bar at all, so its key row is the last thing in the window with the keyboard
    // down as well as up and owes the gesture handle either way.
    @Test
    fun aRotatedPaneHasNothingUnderItWhicheverWayTheKeyboardIs() {
        for (ime in listOf(0.dp, 300.dp)) {
            assertFalse(bottomChrome(Breakpoint.Landscape, Screen.Pane("x", PaneView.Terminal), ime), "ime=$ime")
            assertTrue(bottomChrome(Breakpoint.Landscape, Screen.Herd, ime), "ime=$ime on the herd")
        }
    }

    // The desktop ends in its status strip on every screen there is, keyboard or no keyboard.
    @Test
    fun theDesktopAlwaysEndsInItsStatusStrip() {
        for (screen in listOf(Screen.Pane("x", PaneView.Split), Screen.Herd, Screen.Setup)) {
            for (ime in listOf(0.dp, 300.dp)) assertTrue(bottomChrome(Breakpoint.Desktop, screen, ime), "$screen ime=$ime")
        }
    }
}
