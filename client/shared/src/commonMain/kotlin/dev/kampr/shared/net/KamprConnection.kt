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
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

private const val CAPS_MIN_INTERVAL_MS = 10_000.0
private const val DEFAULT_OWNER = "screen"

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

    var endpoint: Endpoint? = null
        private set

    fun connect(target: Endpoint) {
        scope.launch {
            loop?.cancelAndJoin()
            endpoint = target
            outbox = Channel(Channel.BUFFERED)
            loop = scope.launch { run(target) }
        }
    }

    fun disconnect() {
        scope.launch {
            loop?.cancelAndJoin()
            loop = null
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
        send(ClientMsg.Unwatch(pane))
    }

    fun observedPanes(): Set<String> = watched.keys.toSet()

    private suspend fun run(target: Endpoint) {
        val client = clientFactory()
        val backoff = Backoff()
        try {
            while (scope.isActive) {
                store.status(ConnectionStatus.Connecting)
                val outcome = runCatching { session(client, target) }
                if (outcome.exceptionOrNull() is CancellationException) throw outcome.exceptionOrNull()!!
                store.markStale()
                val wait = backoff.next()
                val reason = outcome.exceptionOrNull()?.message ?: "connection closed"
                store.status(ConnectionStatus.Offline(reason, wait))
                delay(wait)
            }
        } finally {
            client.close()
        }
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
            try {
                capsWanted = false
                capsAskedAt = 0.0
                for (pane in watched.keys) outbox.trySend(ClientMsg.Watch(pane))
                for (frame in incoming) {
                    val text = (frame as? Frame.Text)?.readText() ?: continue
                    val msg = Wire.decode(text) ?: continue
                    if (msg is ServerMsg.Pong) {
                        pingSentAt.remove(msg.n)?.let { store.recordRtt(nowMillis() - it) }
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
        val now = nowMillis()
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

    private suspend fun heartbeat() {
        while (true) {
            delay(10_000)
            val n = ++pingSeq
            // A full outbox is the backlog a heartbeat exists to find, so a dropped ping is left
            // dropped: the pong that never comes closes the socket. Stamping a send that did not
            // happen would only turn the next round trip into a lie about the latency.
            if (outbox.trySend(ClientMsg.Ping(n)).isSuccess) pingSentAt[n] = nowMillis()
            if (pingSentAt.size > 8) pingSentAt.clear()
        }
    }
}
