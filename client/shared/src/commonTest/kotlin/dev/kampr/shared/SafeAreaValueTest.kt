package dev.kampr.shared

import androidx.compose.ui.unit.dp
import dev.kampr.shared.ui.safeAreaOf
import kotlin.test.Test
import kotlin.test.assertEquals

class SafeAreaValueTest {
    // The system keeps reporting the navigation bar while the keyboard is drawn over it. Paying
    // for both is a strip of dead ground between the last control and the keys.
    @Test
    fun anOpenKeyboardTakesOverTheBottomBar() {
        val open = safeAreaOf(top = 32.dp, bottom = 24.dp, left = 0.dp, right = 0.dp, ime = 300.dp)
        assertEquals(0.dp, open.bottom)
        assertEquals(300.dp, open.ime)
        assertEquals(32.dp, open.top)
    }

    @Test
    fun aClosedKeyboardLeavesEveryOtherSideAlone() {
        val shut = safeAreaOf(top = 32.dp, bottom = 24.dp, left = 8.dp, right = 4.dp, ime = 0.dp)
        assertEquals(24.dp, shut.bottom)
        assertEquals(0.dp, shut.ime)
        assertEquals(8.dp, shut.left)
        assertEquals(4.dp, shut.right)
    }

    // Rotated, the bars take a side and the keyboard still only takes the bottom.
    @Test
    fun theSidesAreNotTheKeyboardsToTake() {
        val open = safeAreaOf(top = 24.dp, bottom = 0.dp, left = 48.dp, right = 0.dp, ime = 220.dp)
        assertEquals(48.dp, open.left)
        assertEquals(0.dp, open.right)
    }
}
