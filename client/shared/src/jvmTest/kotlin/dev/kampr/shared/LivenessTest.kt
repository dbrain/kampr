package dev.kampr.shared

import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.net.Liveness
import dev.kampr.shared.wire.ClientMsg
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import java.util.concurrent.atomic.AtomicLong
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

// A device that sleeps: the wall clock counts the sleep and the monotonic one does not, which is
// the whole difference between the two readings the connection has to tell apart.
private class SleepyClocks {
    private val slept = AtomicLong(0)

    fun sleepFor(ms: Long) = slept.addAndGet(ms).let { }

    val monotonic: () -> Double = { System.nanoTime() / 1_000_000.0 }
    val wall: () -> Double = { System.currentTimeMillis().toDouble() + slept.get() }
}

class LivenessTest {
    // The heartbeat's comment promised that "the pong that never comes closes the socket". Nothing
    // closed it. Unanswered pings piled up in a map that was emptied every eight rounds and the
    // read loop stayed parked in `for (frame in incoming)` until the OS finally errored the TCP
    // connection, which on a phone is minutes.
    @Test
    fun aSocketThatHasStoppedAnsweringIsClosedRatherThanLeftLookingLive() {
        val node = StubNode()
        node.deaf = true
        node.start()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(
            scope,
            store,
            liveness = Liveness(
                pingIntervalMs = 200.0,
                silenceDeadlineMs = 700.0,
                // High enough that only the deadline can be what drops the socket here.
                resumeQuietMs = 600_000.0,
                probeMs = 300.0,
                tickMs = 50,
            ),
        )
        try {
            runBlocking {
                connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
                until("hello") { store.hello.value != null }
                until("the unanswered socket to be dropped and re-dialled", timeoutMs = 8_000) {
                    node.accepted.get() >= 3
                }
            }
        } finally {
            connection.disconnect()
            scope.cancel()
            node.stop()
        }
    }

    // The guard on the rule above: a node that is answering must never be dropped, however long
    // the session runs. A deadline that fires on a healthy socket is a reconnect loop.
    @Test
    fun aNodeThatAnswersItsPingsKeepsTheSameSocket() {
        val node = StubNode()
        node.start()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(
            scope,
            store,
            liveness = Liveness(
                pingIntervalMs = 100.0,
                silenceDeadlineMs = 500.0,
                resumeQuietMs = 600_000.0,
                probeMs = 300.0,
                tickMs = 25,
            ),
        )
        try {
            runBlocking {
                connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
                until("hello") { store.hello.value != null }
                delay(3_000)
                assertEquals(
                    1,
                    node.accepted.get(),
                    "a node answering every ping still had its socket dropped",
                )
                assertTrue(store.status.value is ConnectionStatus.Live)
            }
        } finally {
            connection.disconnect()
            scope.cancel()
            node.stop()
        }
    }

    // `nowMillis()` on Android is `System.nanoTime()` — CLOCK_MONOTONIC, which does not advance in
    // deep sleep. A resume that asked it how long the app had been away would be told "no time at
    // all" by the very sleep it is trying to measure, and would leave a dead socket alone. Here the
    // silence deadline is set out of reach on purpose, so the only thing that can drop this socket
    // is the resume reading a clock that counted the sleep.
    @Test
    fun aResumeMeasuresTheBackgroundOnAClockThatCountedTheSleep() {
        val node = StubNode()
        node.start()
        val clocks = SleepyClocks()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(
            scope,
            store,
            liveness = Liveness(
                pingIntervalMs = 200.0,
                silenceDeadlineMs = 600_000.0,
                resumeQuietMs = 12_000.0,
                probeMs = 300.0,
                tickMs = 50,
                monotonic = clocks.monotonic,
                wall = clocks.wall,
            ),
        )
        try {
            runBlocking {
                connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
                until("hello") { store.hello.value != null }
                until("a round trip") { node.received.any { it.contains("\"ping\"") } }

                node.deaf = true
                clocks.sleepFor(300_000)
                connection.onForeground()

                until("the dead socket to be dropped and re-dialled", timeoutMs = 8_000) {
                    node.accepted.get() >= 2
                }
            }
        } finally {
            connection.disconnect()
            scope.cancel()
            node.stop()
        }
    }

    // A five-second glance at a notification is not evidence of anything, and below the quiet
    // window the socket has not even missed a heartbeat. Re-dialling there throws away a socket
    // that is fine and re-fetches every watched pane's scrollback; probing there is a round trip
    // bought for nothing. The ping interval is out of reach so the only thing that could put a
    // frame on the wire is the resume.
    @Test
    fun aResumeAfterAGlanceDoesNotEvenAskTheNode() {
        val node = StubNode()
        node.start()
        val clocks = SleepyClocks()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(
            scope,
            store,
            liveness = Liveness(
                pingIntervalMs = 600_000.0,
                silenceDeadlineMs = 600_000.0,
                resumeQuietMs = 12_000.0,
                probeMs = 300.0,
                tickMs = 25,
                monotonic = clocks.monotonic,
                wall = clocks.wall,
            ),
        )
        try {
            runBlocking {
                connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
                until("hello") { store.hello.value != null }

                clocks.sleepFor(5_000)
                repeat(10) {
                    connection.onForeground()
                    delay(50)
                }
                delay(500)
                assertEquals(
                    0,
                    node.received.count { it.contains("\"ping\"") },
                    "a five-second background cost a round trip the socket had not earned",
                )
                assertEquals(1, node.accepted.get(), "a healthy socket was re-dialled for a glance")
            }
        } finally {
            connection.disconnect()
            scope.cancel()
            node.stop()
        }
    }

    // The other half of the resume rule, and the reason it is one ping rather than a re-dial: a
    // long background is not proof the socket died. A tablet on wifi that was simply not looked at
    // comes back to a socket that still answers, and answering is what keeps it.
    @Test
    fun aResumeAfterALongBackgroundKeepsASocketThatStillAnswers() {
        val node = StubNode()
        node.start()
        val clocks = SleepyClocks()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(
            scope,
            store,
            liveness = Liveness(
                pingIntervalMs = 600_000.0,
                silenceDeadlineMs = 600_000.0,
                resumeQuietMs = 12_000.0,
                probeMs = 300.0,
                tickMs = 25,
                monotonic = clocks.monotonic,
                wall = clocks.wall,
            ),
        )
        try {
            runBlocking {
                connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
                until("hello") { store.hello.value != null }

                clocks.sleepFor(300_000)
                connection.onForeground()
                until("the resume to ask the node") {
                    node.received.any { it.contains("\"ping\"") }
                }
                delay(1_500)
                assertEquals(
                    1,
                    node.accepted.get(),
                    "a socket that answered the resume's ping was dropped anyway",
                )
                assertTrue(store.status.value is ConnectionStatus.Live)
            }
        } finally {
            connection.disconnect()
            scope.cancel()
            node.stop()
        }
    }

    // Reconnecting faster must not turn into replaying stale keystrokes. Whatever ends a session
    // — the node closing it, or this client deciding it is dead — has to leave the outbox through
    // the same door, or a resume replays a half-typed command into a live shell.
    @Test
    fun keystrokesTypedAfterAResumeDropTheSocketAreDiscardedRatherThanReplayed() {
        val node = StubNode()
        node.start()
        val clocks = SleepyClocks()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(
            scope,
            store,
            liveness = Liveness(
                pingIntervalMs = 200.0,
                silenceDeadlineMs = 600_000.0,
                resumeQuietMs = 12_000.0,
                probeMs = 300.0,
                tickMs = 50,
                monotonic = clocks.monotonic,
                wall = clocks.wall,
            ),
        )
        try {
            runBlocking {
                store.pane(PANE)
                connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
                until("hello") { store.hello.value != null }
                connection.watch(PANE)
                connection.send(ClientMsg.InputText(PANE, "MARK"))
                until("the first keystroke") { node.received.any { it.contains("MARK") } }

                // Nothing can reconnect behind the test's back, so what happens next is the
                // resume's doing and nothing else's.
                node.stopListening()
                node.deaf = true
                clocks.sleepFor(300_000)
                connection.onForeground()
                until("the resume to drop the dead socket", timeoutMs = 8_000) {
                    store.status.value !is ConnectionStatus.Live
                }

                repeat(200) { connection.send(ClientMsg.InputText(PANE, "x")) }
                delay(500)
                assertEquals(
                    0,
                    node.received.count { it.contains("\"x\"") },
                    "keystrokes typed after the resume dropped the socket reached the shell anyway",
                )
                assertEquals(
                    200,
                    store.pane(PANE).undelivered,
                    "the drop was never signalled to the pane",
                )
            }
        } finally {
            connection.disconnect()
            scope.cancel()
            node.stop()
        }
    }
}
