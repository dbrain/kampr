package dev.kampr.shared.net

import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.ClientMsg
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

class KamprConnection(
    private val scope: CoroutineScope,
    val store: KamprStore,
    private val clientFactory: () -> HttpClient = ::createHttpClient,
) {
    private var loop: Job? = null
    private var outbox = Channel<ClientMsg>(Channel.BUFFERED)
    private val watched = LinkedHashSet<String>()
    private var pingSeq = 0
    private val pingSentAt = HashMap<Int, Double>()

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
        outbox.trySend(msg)
    }

    fun watch(pane: String, scrollback: Boolean = true, conversation: Boolean = true) {
        if (watched.add(pane)) send(ClientMsg.Watch(pane, scrollback, conversation))
    }

    fun unwatch(pane: String) {
        if (watched.remove(pane)) send(ClientMsg.Unwatch(pane))
    }

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
            val pump = launch { pump(this@webSocket) }
            val heartbeat = launch { heartbeat() }
            try {
                for (pane in watched) outbox.trySend(ClientMsg.Watch(pane))
                for (frame in incoming) {
                    val text = (frame as? Frame.Text)?.readText() ?: continue
                    val msg = Wire.decode(text) ?: continue
                    if (msg is ServerMsg.Pong) {
                        pingSentAt.remove(msg.n)?.let { store.recordRtt(nowMillis() - it) }
                        continue
                    }
                    store.accept(msg)
                }
            } finally {
                heartbeat.cancel()
                pump.cancel()
                runCatching { close() }
            }
        }
    }

    private suspend fun pump(session: DefaultClientWebSocketSession) {
        for (msg in outbox) session.send(Frame.Text(Wire.encode(msg)))
    }

    private suspend fun heartbeat() {
        while (true) {
            delay(10_000)
            val n = ++pingSeq
            pingSentAt[n] = nowMillis()
            outbox.trySend(ClientMsg.Ping(n))
            if (pingSentAt.size > 8) pingSentAt.clear()
        }
    }
}
