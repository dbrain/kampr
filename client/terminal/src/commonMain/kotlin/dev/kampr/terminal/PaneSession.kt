package dev.kampr.terminal

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.terminal.file.FilePeek
import dev.kampr.terminal.file.Handover
import dev.kampr.terminal.guard.ConfirmState
import dev.kampr.terminal.input.Latches
import dev.kampr.terminal.input.KeyTrace
import dev.kampr.terminal.input.LocalEcho
import dev.kampr.terminal.input.ScrollTrace
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

    // A file the operator opened off the grid. Held here rather than in the view that opened it,
    // because a fetch outlives the strip that started it.
    val peek = FilePeek()

    // Where a file the operator handed to this pane has got to. Session-scoped like the peek: the
    // node's refusal can land after the strip that started the paste has been recomposed away.
    var handover by mutableStateOf<Handover>(Handover.Idle)
    val latches = Latches()
    val confirm = ConfirmState()

    // What a gesture asked a program for when Kampr has no ring to move — off unless the entry
    // point turned it on, and one boolean per report when it is.
    val scrollTrace = ScrollTrace()
    val keyTrace = KeyTrace()
    val echo = LocalEcho()

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

    // Measured, not guessed: the strip is absent until something scrolls out of view, and grows
    // with the review bar, the handover line and the type scale.
    var indicatorHeight by mutableStateOf(0f)

    // The pane actions sheet is composed at the app root and never sees the grid, so it asks here
    // and the terminal view on screen carries it out. `onScreen` is how the sheet knows there is
    // one to ask; `attachable` is that view's own verdict on a picker and a device that may type.
    var asked by mutableStateOf<PaneTool?>(null)
    var onScreen by mutableIntStateOf(0)
    var attachable by mutableStateOf(false)

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

enum class PaneTool { Review, Attach }

class PaneSessions {
    private val sessions = HashMap<String, PaneSession>()

    operator fun get(paneId: String): PaneSession = sessions.getOrPut(paneId) { PaneSession(paneId) }
}
