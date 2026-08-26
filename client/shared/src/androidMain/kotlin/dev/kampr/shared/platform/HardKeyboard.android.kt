package dev.kampr.shared.platform

import android.content.res.Configuration
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalConfiguration

// Two readings, and both have to agree.
//
// `keyboard` on its own is not a fact about right now: it is resolved from the device's own
// resource configuration, and a ROM that declares a `qwerty` keyboard reports KEYBOARD_QWERTY with
// nothing attached to it. Believing that alone is a tablet with no Escape key, which is the
// expensive direction.
//
// `hardKeyboardHidden` is the runtime half. The framework holds it at HARDKEYBOARDHIDDEN_NO only
// while a hard keyboard is actually exposed to the person holding the device, and drives it to
// HARDKEYBOARDHIDDEN_YES when the slider shuts or the Bluetooth keyboard goes away. It is also YES
// on a device that has no hard keyboard at all, so the conjunction reads "declared, and available
// this second" and a wrong `keyboard` cannot carry it on its own.
//
// Not `InputDevice`: enumerating SOURCE_KEYBOARD devices is a point-in-time query that needs an
// InputManager.InputDeviceListener behind it to be reactive at all, and it counts virtual and
// vendor devices that report KEYBOARD_TYPE_ALPHABETIC without a key on them. The configuration is
// the signal the framework itself keeps current, and it is what makes this reactive for free:
// attaching a keyboard is a configuration change, and `LocalConfiguration` is recomposed on it.
internal fun hardKeyboardIn(configuration: Configuration): Boolean =
    configuration.keyboard != Configuration.KEYBOARD_NOKEYS &&
        configuration.hardKeyboardHidden == Configuration.HARDKEYBOARDHIDDEN_NO

@Composable
actual fun hardKeyboardAttached(): Boolean = hardKeyboardIn(LocalConfiguration.current)
