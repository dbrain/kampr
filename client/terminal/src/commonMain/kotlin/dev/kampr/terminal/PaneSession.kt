package dev.kampr.terminal

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.terminal.guard.ConfirmState
import dev.kampr.terminal.input.Latches
import dev.kampr.terminal.view.TerminalViewState

// The terminal surface and the key row are separate composables in separate subtrees, so the
// latches, the keyboard request and the zoom they share live here, keyed by pane.
@Stable
class PaneSession(val paneId: String) {
    val view = TerminalViewState()
    val latches = Latches()
    val confirm = ConfirmState()

    var keyboardOpen by mutableStateOf(false)
        private set
    var focusRequests by mutableIntStateOf(0)
        private set
    var keyRowHeight by mutableStateOf(0f)

    // Tapping the grid is the only way in, the way every terminal emulator behaves. A pan or a
    // pinch must not count as a tap, or the keyboard flickers on every flick.
    fun openKeyboard() {
        keyboardOpen = true
        focusRequests++
    }

    fun closeKeyboard() {
        keyboardOpen = false
    }
}

class PaneSessions {
    private val sessions = HashMap<String, PaneSession>()

    operator fun get(paneId: String): PaneSession = sessions.getOrPut(paneId) { PaneSession(paneId) }
}
