package dev.kampr.shared

import dev.kampr.shared.net.createHttpClient
import io.ktor.client.engine.okhttp.OkHttpConfig
import kotlin.test.Test
import kotlin.test.assertTrue

// OkHttp never turns Nagle off, and Android leaves it on, so a keystroke typed while the last one
// was still unacknowledged sat in the phone's own socket until the ACK came back — a round trip,
// or a delayed ACK's 40 ms, on top of everything else a keystroke waits for.
class NoDelayTest {
    @Test
    fun theSocketAKeystrokeLeavesThePhoneOnSendsItTheMomentItIsWritten() {
        val config = createHttpClient().engine.config as OkHttpConfig
        val factory = config.preconfigured?.socketFactory
            ?: error("the client is built on OkHttp's default socket factory, which leaves Nagle on")
        factory.createSocket().use { socket ->
            assertTrue(socket.tcpNoDelay, "a socket the app dials with still has Nagle on")
        }
    }
}
