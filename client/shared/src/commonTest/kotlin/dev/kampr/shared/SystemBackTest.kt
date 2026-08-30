package dev.kampr.shared

import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.backTarget
import dev.kampr.shared.ui.tabFor
import dev.kampr.shared.ui.Tab
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

private val PANE = Screen.Pane("01JNODE/w1:p1", PaneView.Terminal)

private val EVERY_SCREEN = listOf(
    Screen.Herd,
    Screen.Mosaic,
    Screen.Fleet,
    PANE,
    Screen.Setup,
    Screen.Devices,
    Screen.Appearance,
    Screen.Notifications,
)

// Android gives the window one gesture for "out of here" and finishes the activity when nothing
// claims it. Every screen below is one a phone can be standing on with a Back control in its own
// chrome, and every one of them used to close Kampr instead.
class SystemBackTest {
    private val phones = listOf(Breakpoint.Portrait, Breakpoint.Landscape)

    @Test
    fun theOneScreenBackIsAllowedToLeaveTheAppFromIsTheHerd() {
        for (breakpoint in phones) {
            assertNull(backTarget(Screen.Herd, breakpoint, paired = true), "$breakpoint")
            for (screen in EVERY_SCREEN - Screen.Herd) {
                assertEquals(
                    true,
                    backTarget(screen, breakpoint, paired = true) != null,
                    "$screen on $breakpoint has nowhere to go, so back closes the app",
                )
            }
        }
    }

    // The report: a terminal on a phone. It is the screen the app is used on, and the one whose
    // back gesture was a one-way door out.
    @Test
    fun backOffAPaneLandsOnTheHerd() {
        for (breakpoint in phones) {
            assertEquals(Screen.Herd, backTarget(PANE, breakpoint, paired = true), "$breakpoint")
        }
    }

    // The wide layout opens the herd's first pane by itself, so a back that landed on the herd
    // would be undone in the frame after it — and the sidebar was never covered to begin with.
    @Test
    fun theWideLayoutHasNoPaneToBackOutOf() {
        assertNull(backTarget(PANE, Breakpoint.Desktop, paired = true))
    }

    // Back walks the ladder it was led down: the settings rungs go up to Setup, never straight out.
    @Test
    fun aSettingsRungLeadsBackToSettings() {
        for (screen in listOf(Screen.Devices, Screen.Appearance, Screen.Notifications)) {
            assertEquals(Screen.Setup, backTarget(screen, Breakpoint.Portrait, paired = true), "$screen")
        }
    }

    // A tab's own root leads to the other tab's root, which is where Android expects back to land.
    @Test
    fun aTabRootLeadsToTheOtherTabsRoot() {
        assertEquals(Tab.Settings, tabFor(Screen.Setup))
        assertEquals(Screen.Herd, backTarget(Screen.Setup, Breakpoint.Portrait, paired = true))
    }

    // Except on a device with nothing to connect with: `AppState` opens on Setup precisely because
    // there is no herd to fetch, and sending back there would strand a first run on an empty list.
    @Test
    fun anUnpairedDeviceIsNotSentToAHerdItCannotFetch() {
        for (breakpoint in phones) {
            assertNull(backTarget(Screen.Setup, breakpoint, paired = false), "$breakpoint")
        }
    }

    // Neither of the herd's other two places is a rung under it, and both were reached from it.
    @Test
    fun theMosaicAndAFleetRunLeadBackToTheHerd() {
        for (screen in listOf(Screen.Mosaic, Screen.Fleet)) {
            assertEquals(Screen.Herd, backTarget(screen, Breakpoint.Portrait, paired = true), "$screen")
        }
    }
}
