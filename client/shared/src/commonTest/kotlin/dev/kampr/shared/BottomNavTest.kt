package dev.kampr.shared

import dev.kampr.shared.ui.Breakpoint
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

    // A pane owns the bottom of the window in every posture: `onBack` leads out, and the tabs are
    // for the screens that are not a pane.
    @Test
    fun aPaneHasNothingOfTheAppsOwnUnderItInEitherPosture() {
        for (breakpoint in listOf(Breakpoint.Portrait, Breakpoint.Landscape)) {
            for (view in PaneView.entries) {
                assertFalse(bottomChrome(breakpoint, Screen.Pane("x", view)), "$breakpoint: $view wears a tab bar")
            }
        }
    }

    // Everywhere else the keyboard is over a scrolling form, and losing the tabs would strand a
    // reader who opened one by accident.
    @Test
    fun everyOtherScreenKeepsItsTabs() {
        for (breakpoint in listOf(Breakpoint.Portrait, Breakpoint.Landscape)) {
            for (screen in listOf(Screen.Herd, Screen.Setup, Screen.Devices, Screen.Appearance, Screen.Notifications)) {
                assertTrue(bottomChrome(breakpoint, screen), "$breakpoint: $screen has no tab bar")
            }
        }
    }

    // The desk ends in whatever screen is open: what its status strip said is in the sidebar, and a
    // held pane says so in its own header.
    @Test
    fun theDeskHasNothingUnderAnyScreen() {
        for (screen in EVERY_SCREEN) assertFalse(bottomChrome(Breakpoint.Desktop, screen), "$screen")
    }
}
