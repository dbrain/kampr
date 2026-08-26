package dev.kampr.terminal

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.terminal.guard.ConfirmState
import dev.kampr.terminal.input.Latches
import dev.kampr.terminal.review.ReviewState
import dev.kampr.terminal.view.GridProbe
import dev.kampr.terminal.view.TerminalViewState

// The terminal surface and the key row are separate composables in separate subtrees, so the
// latches, the keyboard request and the zoom they share live here, keyed by pane.
@Stable
class PaneSession(val paneId: String) {
    val view = TerminalViewState()

    // Where the grid was last painted. A gesture detector reads it to turn a finger into a cell,
    // and it is the only place the painted geometry of a frame is a value rather than a local.
    val grid = GridProbe()
    val review = ReviewState()
    val latches = Latches()
    val confirm = ConfirmState()

    var keyboardOpen by mutableStateOf(false)
        private set
    var focusRequests by mutableIntStateOf(0)
        private set

    // Every gesture that ends on the pane surface, asked-for keyboard or not. A browser holding
    // focus for a hardware keyboard never asks for one, so this is the only cue it gets that the
    // pointer down took the offscreen input's focus away and it is time to take it back.
    var surfaceSettled by mutableIntStateOf(0)
        private set
    var keyRowHeight by mutableStateOf(0f)

    // Measured, not guessed: the strip grows with the review pill, the type scale and the
    // review bar, and a constant that is short by any of them paints the prompt behind it.
    var indicatorHeight by mutableStateOf(0f)

    // Tapping the grid is the only way in, the way every terminal emulator behaves. A pan or a
    // pinch must not count as a tap, or the keyboard flickers on every flick.
    fun openKeyboard() {
        keyboardOpen = true
        focusRequests++
    }

    fun closeKeyboard() {
        keyboardOpen = false
    }

    fun toggleKeyboard() {
        if (keyboardOpen) closeKeyboard() else openKeyboard()
    }

    // A browser blurs the offscreen input on any pointer down on the canvas, the key row included,
    // so focus is claimed back once the gesture is over or the next keystrokes go nowhere.
    fun reclaimKeyboard() {
        surfaceSettled++
        if (keyboardOpen) focusRequests++
    }
}

class PaneSessions {
    private val sessions = HashMap<String, PaneSession>()

    operator fun get(paneId: String): PaneSession = sessions.getOrPut(paneId) { PaneSession(paneId) }
}
