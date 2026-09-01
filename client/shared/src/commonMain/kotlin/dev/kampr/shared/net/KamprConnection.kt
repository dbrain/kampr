package dev.kampr.shared.net

import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import io.ktor.client.HttpClient
import io.ktor.client.plugins.websocket.DefaultClientWebSocketSession
import io.ktor.client.plugins.websocket.webSocket
import io.ktor.client.request.header
import io.ktor.websocket.Frame
import io.ktor.websocket.close
import io.ktor.websocket.readText
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

private const val CAPS_MIN_INTERVAL_MS = 10_000.0

// How long a socket has to stay open before the retry ladder is walked back down to its first
// step. Long enough that a node crash-looping on start cannot pass for health at any rung of the
// ladder, short enough that an ordinary session — a phone changing network, a laptop waking —
// always does.
private const val HEALTHY_SESSION_MS = 5_000.0
private const val DEFAULT_OWNER = "screen"

private const val REFUSED =
    "This node no longer recognises this device. It was removed, or the node was set up again. " +
        "Pair it once more to get back in."

// Keystrokes are addressed to the shell in front of the operator now. A queue that outlives the
// socket replays a half-typed command into a live shell a reconnect later — measured at twenty
// seconds — so input is dropped the moment there is nowhere to put it and the pane says so.
// Everything else here is a standing intent that is still true after a reconnect and keeps its
// place in the queue.
private val ClientMsg.typing: String?
    get() = when (this) {
        is ClientMsg.InputText -> pane
        is ClientMsg.InputB64 -> pane
        is ClientMsg.InputKeys -> pane
        is ClientMsg.Answer -> pane
        else -> null
    }

class KamprConnection(
    private val scope: CoroutineScope,
    val store: KamprStore,
    private val clientFactory: () -> HttpClient = ::createHttpClient,
    private val liveness: Liveness = Liveness(),
) {
    private var loop: Job? = null
    private var outbox = Channel<ClientMsg>(Channel.BUFFERED)
    private var live = false
    // A pane can be on screen twice at once — the pane screen and any number of mosaic cells —
    // and the last viewer to let go is the one that stops the stream. Watching by owner is what
    // makes "removing a cell unwatches" true without it also unwatching what is still on screen.
    private val watched = LinkedHashMap<String, MutableSet<String>>()
    private var pingSeq = 0
    private val pingSentAt = HashMap<Int, Double>()
    private var capsWanted = false
    private var capsAskedAt = 0.0
    private var openedAt = 0.0
    private var heardAt = 0.0
    private var heardAtWall = 0.0
    private var probeUntil = 0.0
    private var probeArmed = false
    private var sessionJob: Job? = null
    private var dropped: String? = null
    // Conflated, and only ever read by whichever of the two waits is the live one: the heartbeat
    // while there is a socket, the retry ladder while there is not.
    private val wake = Channel<Unit>(Channel.CONFLATED)
    private var foreground: ForegroundWatch? = null

    var endpoint: Endpoint? = null
        private set

    fun connect(target: Endpoint) {
        scope.launch {
            loop?.cancelAndJoin()
            endpoint = target
            outbox = Channel(Channel.BUFFERED)
            if (foreground == null) foreground = watchForeground(::onForeground)
            loop = scope.launch { run(target) }
        }
    }

    fun disconnect() {
        scope.launch {
            loop?.cancelAndJoin()
            loop = null
            foreground?.stop()
            foreground = null
            store.status(ConnectionStatus.Idle)
        }
    }

    fun send(msg: ClientMsg) {
        val pane = msg.typing
        val delivered = (live || pane == null) && outbox.trySend(msg).isSuccess
        if (pane != null) store.noteInput(pane, delivered)
    }

    fun manage(op: ManageOp) {
        send(ClientMsg.Manage(op))
    }

    fun watch(
        pane: String,
        owner: String = DEFAULT_OWNER,
        scrollback: Boolean = true,
        conversation: Boolean = true,
    ) {
        val owners = watched.getOrPut(pane) { LinkedHashSet() }
        val first = owners.isEmpty()
        owners += owner
        if (first) send(ClientMsg.Watch(pane, scrollback, conversation))
    }

    fun unwatch(pane: String, owner: String = DEFAULT_OWNER) {
        val owners = watched[pane] ?: return
        if (!owners.remove(owner) || owners.isNotEmpty()) return
        watched.remove(pane)
        store.noteConversationUnconfirmed(pane)
        send(ClientMsg.Unwatch(pane))
    }

    fun observedPanes(): Set<String> = watched.keys.toSet()

    // The one measurement that has to span a background, so the one that cannot use the monotonic
    // clock: `nowMillis()` is CLOCK_MONOTONIC on Android and stops dead in deep sleep, and asking
    // it how long the app was away gets the answer "no time at all" from the very sleep being
    // measured. The wall clock counts it. It can also step, so a reading that came out negative is
    // read as "cannot tell" and probes rather than trusted.
    //
    // Below the quiet window the socket has not even missed a heartbeat and is left alone: a glance
    // at a notification must not cost every watched pane its scrollback. Above it, one ping decides
    // it — a socket that is alive answers in a round trip and keeps everything it had.
    fun onForeground() {
        val silence = liveness.wall() - heardAtWall
        if (live && silence >= 0.0 && silence < liveness.resumeQuietMs) return
        probeArmed = true
        wake.trySend(Unit)
    }

    private suspend fun run(target: Endpoint) {
        val client = clientFactory()
        val backoff = Backoff()
        try {
            while (scope.isActive) {
                store.status(ConnectionStatus.Connecting)
                openedAt = 0.0
                dropped = null
                val outcome = runCatching { attempt(client, target) }
                if (outcome.exceptionOrNull() is CancellationException) throw outcome.exceptionOrNull()!!
                store.markStale()
                // A session that stayed up is the only evidence there is that this address works,
                // and it is what the ladder exists to find. Without it the delay only ever grew:
                // a phone that lost its network a handful of times over a day waited the full
                // ceiling before every attempt after that.
                if (openedAt != 0.0 && liveness.wall() - openedAt >= HEALTHY_SESSION_MS) backoff.reset()
                val wait = backoff.next()
                val reason = dropped ?: outcome.exceptionOrNull()?.message ?: "connection closed"
                // A socket that will not open says nothing about why. Asking the same token over
                // HTTP is what separates a node that is down — which comes back on its own and must
                // keep saying "reconnecting" — from one that has been reset, or has revoked this
                // device, which never will. The retry continues either way, so a node whose
                // database is restored still heals without a reload.
                if (target.token != null && NodeApi(client, target).refusesToken()) {
                    store.status(ConnectionStatus.Refused(REFUSED))
                } else {
                    store.status(ConnectionStatus.Offline(reason, wait))
                }
                waitOrWake(wait)
            }
        } finally {
            client.close()
        }
    }

    // A socket this client decided was dead has to end without a `CancellationException` reaching
    // the ladder above, which reads one as the whole connection being torn down. Cancelling a child
    // job is not a failure of its parent, so the join returns and the ladder walks on.
    private suspend fun attempt(client: HttpClient, target: Endpoint) = coroutineScope {
        val job = launch { session(client, target) }
        sessionJob = job
        job.join()
    }

    private fun drop(reason: String) {
        dropped = reason
        sessionJob?.cancel()
    }

    private suspend fun waitOrWake(ms: Long) {
        withTimeoutOrNull(ms) { wake.receive() }
    }

    private suspend fun session(client: HttpClient, target: Endpoint) {
        client.webSocket(
            urlString = target.wsUrl,
            request = { target.subprotocol?.let { header("Sec-WebSocket-Protocol", it) } },
        ) {
            val queue = outbox
            val pump = launch { pump(this@webSocket, queue) }
            val heartbeat = launch { heartbeat() }
            live = true
            openedAt = liveness.wall()
            heard()
            try {
                capsWanted = false
                capsAskedAt = 0.0
                probeArmed = false
                while (wake.tryReceive().isSuccess) { }
                for (pane in watched.keys) outbox.trySend(ClientMsg.Watch(pane))
                for (frame in incoming) {
                    heard()
                    val text = (frame as? Frame.Text)?.readText() ?: continue
                    val msg = Wire.decode(text) ?: continue
                    if (msg is ServerMsg.Pong) {
                        pingSentAt.remove(msg.n)?.let { store.recordRtt(liveness.monotonic() - it) }
                        continue
                    }
                    store.accept(msg)
                    when (msg) {
                        is ServerMsg.Hello -> {
                            capsWanted = msg.caps.manage
                            askCaps()
                        }
                        // Agent kinds and named sessions both move when the herd does, and the
                        // node caches its answer anyway, so a patch is the cue to re-read them.
                        is ServerMsg.Herd, is ServerMsg.HerdPatch -> askCaps()
                        else -> Unit
                    }
                }
            } finally {
                live = false
                heartbeat.cancel()
                pump.cancel()
                discardTyping(queue)
                runCatching { close() }
            }
        }
    }

    // The node caches `caps` for ten seconds because one half of it costs a process; asking more
    // often than that only turns a burst of herd patches into a burst of no-ops.
    private fun askCaps() {
        if (!capsWanted) return
        val now = liveness.monotonic()
        if (capsAskedAt != 0.0 && now - capsAskedAt < CAPS_MIN_INTERVAL_MS) return
        capsAskedAt = now
        send(ClientMsg.RequestCaps)
    }

    private suspend fun pump(session: DefaultClientWebSocketSession, queue: Channel<ClientMsg>) {
        for (msg in queue) session.send(Frame.Text(Wire.encode(msg)))
    }

    // What was queued a frame before the socket died is as stale as what was typed after it, so
    // the standing intents are put back and the keystrokes are not.
    private fun discardTyping(queue: Channel<ClientMsg>) {
        val kept = mutableListOf<ClientMsg>()
        while (true) {
            val msg = queue.tryReceive().getOrNull() ?: break
            val pane = msg.typing
            if (pane == null) kept += msg else store.noteInput(pane, delivered = false)
        }
        for (msg in kept) queue.trySend(msg)
    }

    private fun heard() {
        heardAt = liveness.monotonic()
        heardAtWall = liveness.wall()
        probeUntil = 0.0
    }

    // The monotonic clock, and deliberately: every deadline here is counted in time this loop was
    // actually running, so a device that slept between two ticks is not accused of having a dead
    // socket, and a wall clock that steps cannot close a socket that is answering. It also means
    // this loop can never detect a background — nothing here runs while the process is frozen,
    // which is what `onForeground` is for.
    private suspend fun heartbeat() {
        var pingedAt = liveness.monotonic()
        while (true) {
            waitOrWake(liveness.tickMs)
            val now = liveness.monotonic()
            if (probeArmed) {
                probeArmed = false
                probeUntil = now + liveness.probeMs
                ping()
            }
            if (now - pingedAt >= liveness.pingIntervalMs) {
                pingedAt = now
                ping()
            }
            if (now - heardAt >= liveness.silenceDeadlineMs) {
                return drop("no reply for ${((now - heardAt) / 1000).toInt()}s")
            }
            if (probeUntil != 0.0 && now >= probeUntil) {
                return drop("no reply since the app came back")
            }
        }
    }

    // A full outbox is the backlog a heartbeat exists to find, so a dropped ping is left dropped.
    // Stamping a send that did not happen would only turn the next round trip into a lie about the
    // latency, and the silence deadline above is what notices either way.
    private fun ping() {
        val n = ++pingSeq
        if (outbox.trySend(ClientMsg.Ping(n)).isSuccess) pingSentAt[n] = liveness.monotonic()
        if (pingSentAt.size > 8) pingSentAt.clear()
    }
}
