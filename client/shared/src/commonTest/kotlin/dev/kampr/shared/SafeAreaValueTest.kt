package dev.kampr.shared

import androidx.compose.ui.unit.dp
import dev.kampr.shared.ui.bottomUnderKeyboard
import kotlin.test.Test
import kotlin.test.assertEquals

class SafeAreaValueTest {
    // The system keeps reporting the navigation bar while the keyboard is drawn over it. Paying
    // for both is a strip of dead ground between the last control and the keys.
    @Test
    fun anOpenKeyboardTakesOverTheBottomBar() {
        assertEquals(0.dp, bottomUnderKeyboard(bottom = 24.dp, ime = 300.dp))
    }

    @Test
    fun aClosedKeyboardTakesNothingAtAll() {
        assertEquals(24.dp, bottomUnderKeyboard(bottom = 24.dp, ime = 0.dp))
    }

    // The half of the rule a switch could not express: the keys arrive over the handle before they
    // are over anything else, and they leave it the same way. A step here is the bottom of the app
    // moving a whole gesture handle in one frame.
    @Test
    fun aKeyboardHalfwayOffTheHandleHasTakenHalfOfIt() {
        assertEquals(14.dp, bottomUnderKeyboard(bottom = 46.dp, ime = 32.dp))
        assertEquals(46.dp, bottomUnderKeyboard(bottom = 46.dp, ime = 0.dp))
        assertEquals(0.dp, bottomUnderKeyboard(bottom = 46.dp, ime = 46.dp))
    }
}
