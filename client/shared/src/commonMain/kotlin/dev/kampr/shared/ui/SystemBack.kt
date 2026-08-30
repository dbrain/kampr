package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.backhandler.BackHandler

// Where the system's back gesture leads, or null where it must be left alone and allowed to leave
// the app. Android hands one gesture to the whole window and finishes the activity when nothing
// claims it, so every screen that draws a Back control owes the gesture the same move — without
// this, the pane a phone spends all of its time on was a one-way door out of Kampr.
//
// `paired` is the same fact `AppState` opens on: a device with nothing to connect with has no herd
// to be sent to, so its Setup screen is the root of the app rather than a rung above one.
internal fun backTarget(screen: Screen, breakpoint: Breakpoint, paired: Boolean): Screen? = when (screen) {
    // The wide layout opens the herd's first pane by itself, so a back that landed on the herd
    // would be undone in the frame after it — and the sidebar means the pane was never covering
    // anything to get back to.
    is Screen.Pane -> Screen.Herd.takeIf { breakpoint != Breakpoint.Desktop }
    Screen.Mosaic, Screen.Fleet -> Screen.Herd
    Screen.Devices, Screen.Appearance, Screen.Notifications -> Screen.Setup
    Screen.Setup -> Screen.Herd.takeIf { paired }
    Screen.Herd -> null
}

// A sheet floats over whatever screen is showing and everything it acts on is behind it, so it is
// what back closes first — the screen underneath has not been left yet.
@OptIn(ExperimentalComposeUiApi::class)
@Composable
internal fun SystemBack(state: AppState, breakpoint: Breakpoint) {
    val target = backTarget(state.screen, breakpoint, state.endpoint?.token != null)
    BackHandler(enabled = state.sheet != null || target != null) {
        when {
            state.sheet != null -> state.closeSheet()
            target != null -> state.go(target)
        }
    }
}
