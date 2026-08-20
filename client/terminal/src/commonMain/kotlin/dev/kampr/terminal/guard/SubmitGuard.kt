package dev.kampr.terminal.guard

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.ui.PaneIo
import dev.kampr.terminal.render.LogicalText
import dev.kampr.terminal.render.SurfaceRows

data class HeldSubmit(val reason: String, val command: String, val payload: String, val paste: Boolean)

// The terminal surface and the key row are separate subtrees with a sink each, so the held submit
// lives on the session they share rather than on either sink.
@Stable
class ConfirmState {
    var held by mutableStateOf<HeldSubmit?>(null)

    // null follows the node's stored pref; a value is the operator's choice in this session, held
    // optimistically because the pref round-trips through the node before it comes back.
    var local by mutableStateOf<Boolean?>(null)
}

// Kampr sends every keystroke live, so there is no composed string to inspect — by the time
// `rm -rf /` has been seen it is already in the PTY. The hook is therefore the submit, and the
// thing inspected is the pane's own echo of the line the cursor sits on.
class SubmitGuard(private val pane: PaneState, private val io: PaneIo, val state: ConfirmState) {
    private val rows = SurfaceRows(pane)
    private val logical = LogicalText(rows)

    fun wanted(): Boolean = state.local ?: io.prefs(pane.id).confirm

    // An agent pane is typed *at*, not driven: `rm -rf` in a Claude prompt box is a description of
    // a command, and confirming it there would make the guard infuriating in the pane an operator
    // spends most of their time in.
    fun armed(): Boolean = wanted() && io.info(pane.id)?.agent == null

    // Everything left of the cursor on the cursor's logical line is what Enter is about to run.
    // lineAt is the copy path's joiner, so a command soft-wrapped across the grid edge arrives as
    // one string rather than two half-commands.
    fun commandLine(): String {
        val (line, offset) = logical.lineAt(rows.historyRows + pane.cursor.row)
        return line.take((offset + pane.cursor.col).coerceIn(0, line.length))
    }

    fun clear() {
        state.held = null
    }

    fun hold(text: String, payload: String, paste: Boolean): Boolean {
        if (!armed()) return false
        val hit = destructiveLine(text) ?: return false
        state.held = HeldSubmit(hit.reason, hit.command, payload, paste)
        return true
    }
}
