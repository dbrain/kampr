package dev.kampr.shared

import androidx.compose.ui.unit.dp
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.breakpointOf
import kotlin.test.Test
import kotlin.test.assertEquals

class BreakpointTest {
    @Test
    fun artboardSizesPickTheirOwnLayout() {
        assertEquals(Breakpoint.Desktop, breakpointOf(1440.dp, 900.dp))
        assertEquals(Breakpoint.Portrait, breakpointOf(390.dp, 844.dp))
        assertEquals(Breakpoint.Landscape, breakpointOf(844.dp, 390.dp))
    }

    @Test
    fun aLargePhoneInLandscapeIsNotADesktop() {
        // Medium_Phone_API_35: 2400x1080 at density 2.625.
        assertEquals(Breakpoint.Landscape, breakpointOf(914.dp, 411.dp))
        assertEquals(Breakpoint.Portrait, breakpointOf(411.dp, 914.dp))
    }

    @Test
    fun tabletsAndSmallWindowsLandWhereExpected() {
        assertEquals(Breakpoint.Desktop, breakpointOf(1024.dp, 768.dp))
        assertEquals(Breakpoint.Landscape, breakpointOf(1180.dp, 500.dp))
        assertEquals(Breakpoint.Portrait, breakpointOf(768.dp, 1024.dp))
        assertEquals(Breakpoint.Portrait, breakpointOf(600.dp, 600.dp))
    }
}
