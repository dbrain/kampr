package dev.kampr.shared.ui

import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.SizeMode
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

// How long a matched hold outlives the view that asked for it.
//
// **A release is a resize.** It puts the pane back to the geometry it was found at (ADR 0013,
// point 3), so a terminal view leaving the composition and another arriving — which is the whole
// of a pane switch — wrote one geometry onto the pane being left and another onto the pane being
// opened, and switching back wrote both again the other way round. The operator, on 0.1.57:
// *"wasm desktop matches the view when its open - switching panes now bounces around"*.
//
// Long enough that going to look at a conversation, a second pane or the herd and coming back
// costs nothing at all; short enough that a pane genuinely left behind is given up while the
// operator is still at the machine that took it. It is not a ceiling on the hold — the lease's
// ceiling is the socket and stays the socket — it is a ceiling on how long an *ended* view goes on
// holding one.
const val MATCH_LINGER_MS = 20_000L

// The `match` holds this client session is carrying, across the views that come and go asking for
// them. ADR 0013's lease is owned by the websocket session at the node; this is the same ownership
// on this side of the wire, and it exists because a Compose view is not a session.
//
// Two rules, and both are about not writing a geometry nobody asked for. A pane already held at
// exactly this grid is not claimed again — a re-claim supersedes the controller and herdr shows the
// desk's own geometry in the gap between the two. And a release waits [`MATCH_LINGER_MS`], so a
// pane handed straight back never let go in the first place.
class MatchHolds(
    private val scope: CoroutineScope,
    private val send: (ClientMsg) -> Unit,
) {
    private val held = mutableMapOf<String, Pair<Int, Int>>()
    private val letting = mutableMapOf<String, Job>()

    fun claim(paneId: String, cols: Int, rows: Int) {
        letting.remove(paneId)?.cancel()
        if (held[paneId] == cols to rows) return
        held[paneId] = cols to rows
        send(ClientMsg.Manage(ManageOp.PaneSize(paneId, cols, rows, SizeMode.Match)))
    }

    // `linger` is false where the operator said so rather than where a view ended: ticking the
    // switch off is an answer about this pane and it is owed the pane back at once.
    fun release(paneId: String, linger: Boolean = true) {
        letting.remove(paneId)?.cancel()
        if (!linger) {
            letGo(paneId)
            return
        }
        letting[paneId] = scope.launch {
            delay(MATCH_LINGER_MS)
            letting.remove(paneId)
            letGo(paneId)
        }
    }

    // The socket went, and the node let go of every lease on it as it did — including restoring
    // each pane. Believing otherwise here would leave a pane the node has already given back
    // recorded as held, and the next view of it would then claim nothing.
    fun disconnected() {
        letting.values.forEach(Job::cancel)
        letting.clear()
        held.clear()
    }

    private fun letGo(paneId: String) {
        if (held.remove(paneId) == null) return
        send(ClientMsg.Manage(ManageOp.PaneSize(paneId, mode = SizeMode.Release)))
    }
}
