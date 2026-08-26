package dev.kampr.shared

import android.content.res.Configuration
import dev.kampr.shared.platform.hardKeyboardIn
import kotlin.test.Test
import kotlin.test.assertEquals

private fun configuration(keyboard: Int, hidden: Int) = Configuration().also {
    it.keyboard = keyboard
    it.hardKeyboardHidden = hidden
}

// Runs on the host JVM against a plain `Configuration`, which is a value object with public fields
// and no framework behind it — so the truth table is testable without a device, and the device is
// only needed for the half this cannot answer: whether the framework moves these two fields when a
// keyboard is plugged in.
class HardKeyboardTest {
    @Test
    fun aDeviceThatDeclaresNoKeysIsNeverReadAsHavingAKeyboard() {
        for (hidden in listOf(Configuration.HARDKEYBOARDHIDDEN_NO, Configuration.HARDKEYBOARDHIDDEN_YES)) {
            assertEquals(
                false,
                hardKeyboardIn(configuration(Configuration.KEYBOARD_NOKEYS, hidden)),
                "a device reporting KEYBOARD_NOKEYS was given a keyboard by hardKeyboardHidden=$hidden",
            )
        }
    }

    // The reading that cannot be trusted on its own: a ROM declaring `qwerty` in its configuration
    // reports KEYBOARD_QWERTY with nothing attached, and believing it is a tablet with no Escape.
    @Test
    fun aDeclaredKeyboardThatIsNotAvailableRightNowDoesNotCountAsOne() {
        for (declared in listOf(Configuration.KEYBOARD_QWERTY, Configuration.KEYBOARD_12KEY)) {
            assertEquals(
                false,
                hardKeyboardIn(configuration(declared, Configuration.HARDKEYBOARDHIDDEN_YES)),
                "keyboard=$declared with the hard keyboard hidden was read as a keyboard on the desk",
            )
        }
    }

    @Test
    fun aKeyboardThatIsDeclaredAndExposedIsTheOneCaseThatCountsAsAttached() {
        assertEquals(
            true,
            hardKeyboardIn(configuration(Configuration.KEYBOARD_QWERTY, Configuration.HARDKEYBOARDHIDDEN_NO)),
            "a qwerty keyboard the framework says is exposed was not believed",
        )
    }

    // The default a `Configuration` arrives at before anything fills it in. It has to fall on the
    // side that keeps the key row, because "not filled in" is not "there is a keyboard".
    @Test
    fun anUndeterminedConfigurationKeepsTheKeyRow() {
        assertEquals(
            false,
            hardKeyboardIn(configuration(Configuration.KEYBOARD_UNDEFINED, Configuration.HARDKEYBOARDHIDDEN_UNDEFINED)),
            "an undetermined configuration was read as a desk",
        )
    }
}
