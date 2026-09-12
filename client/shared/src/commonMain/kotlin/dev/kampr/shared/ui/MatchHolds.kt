package dev.kampr.shared.ui

import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.SizeMode
import kotlinx.coroutines.CompletableDeferred
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
// Three rules, and the first two are about not writing a geometry nobody asked for. A pane already
// held at exactly this grid is not claimed again — a re-claim supersedes the controller and herdr
// shows the desk's own geometry in the gap between the two. A release waits [`MATCH_LINGER_MS`], so
// a pane handed straight back never let go in the first place.
//
// **And a pane is held when the node says it is, never when this side sent the ask.** A claim is
// the one op ADR 0012 lets reshape a pane and it can be refused — a controller herdr will not give
// up (#21), a peer link that dropped mid-op, a pane that stopped existing — and a refusal recorded
// as a hold is a client that will not ask again, because the ask it will not repeat is the one that
// never landed. The operator, on 0.1.80: *"i closed it the pane redrew and made claude some tiny
// little box ... didn't seem to recover until i manually set the size"*. There is no re-measuring
// loop behind this and there must not be one — a claim is edge-triggered by the view and never by
// what the node reports about the pane, which is what keeps two viewers from alternating
// (ADR 0013 point 2) — so the only honest answer to a refusal is to forget it and ask again.
class MatchHolds(
    private val scope: CoroutineScope,
    private val send: (ClientMsg) -> Unit,
) {
    private val held = mutableMapOf<String, Pair<Int, Int>>()
    private val asking = mutableMapOf<String, Asking>()
    private val letting = mutableMapOf<String, Job>()
    private var asks = 0L

    private class Asking(
        val paneId: String,
        val size: Pair<Int, Int>,
        val answer: CompletableDeferred<Boolean>,
    )

    // Answers whether the node took the pane, which is not the same question as whether this view
    // asked for it.
    //
    // **There was a slack test here and it stopped panes resizing at all.** It declined a claim
    // whose view was already close to the pane's current geometry — measured against what the pane
    // reads *now*, which in the window right after a release is still the size the hold had put on
    // it. So the first ask after a release answered "close enough", declined, and nothing ever
    // asked again; the restore then took the pane back to its own geometry and left it there. The
    // number to judge against is the geometry the pane has when nothing of Kampr's is on it, which
    // this side of the wire does not know — the node's match ack carries it as `found_cols`/
    // `found_rows`, and that is where a slack test would have to live.
    suspend fun claim(paneId: String, cols: Int, rows: Int): Boolean {
        letting.remove(paneId)?.cancel()
        val size = cols to rows
        asking.values.find { it.paneId == paneId && it.size == size }?.let { return it.answer.await() }
        if (held[paneId] == size) return true
        val asked = Asking(paneId, size, CompletableDeferred())
        val rid = "match-${++asks}"
        asking[rid] = asked
        send(ClientMsg.Manage(ManageOp.PaneSize(paneId, cols, rows, SizeMode.Match), rid))
        return asked.answer.await()
    }

    // `linger` is false where the operator said so rather than where a view ended: ticking the
    // switch off is an answer about this pane and it is owed the pane back at once.
    fun release(paneId: String, linger: Boolean = true) {
        letting.remove(paneId)?.cancel()
        if (!linger) {
            letGo(paneId)
            return
        }
        // The job checks it is still the one registered before it lets go. `cancel` is only an
        // answer while the delay is still suspended: a timer that has already fired has resumed a
        // continuation nothing can take back, and there is no suspension point between there and
        // the release — so a claim arriving in that window cancelled a job that went on to release
        // the pane it had just taken.
        lateinit var job: Job
        job = scope.launch {
            delay(MATCH_LINGER_MS)
            if (letting[paneId] !== job) return@launch
            letting.remove(paneId)
            letGo(paneId)
        }
        letting[paneId] = job
    }

    // The node's answer to one ask, named by the token it was sent with. A refusal is forgotten
    // rather than remembered as a hold: the view that asked is still open and still the right
    // size, and it is the one that asks again.
    fun acked(ack: ServerMsg.Managed) {
        val asked = asking.remove(ack.rid ?: return) ?: return
        if (ack.ok) held[asked.paneId] = asked.size else held.remove(asked.paneId)
        asked.answer.complete(ack.ok)
    }

    // The socket went, and the node let go of every lease on it as it did — including restoring
    // each pane. Believing otherwise here would leave a pane the node has already given back
    // recorded as held, and the next view of it would then claim nothing.
    fun disconnected() {
        letting.values.forEach(Job::cancel)
        letting.clear()
        held.clear()
        asking.values.forEach { it.answer.complete(false) }
        asking.clear()
    }

    // A claim still in flight is let go of too, and the node answers ops in the order they were
    // sent: the release lands on the hold the claim took rather than on nothing. Waiting for the
    // ack first would leave a view that ended mid-claim holding a pane with nothing but the socket
    // to end it.
    private fun letGo(paneId: String) {
        val inFlight = asking.filterValues { it.paneId == paneId }.keys.toList()
        inFlight.forEach { asking.remove(it)?.answer?.complete(false) }
        if (held.remove(paneId) == null && inFlight.isEmpty()) return
        send(ClientMsg.Manage(ManageOp.PaneSize(paneId, mode = SizeMode.Release)))
    }
}
