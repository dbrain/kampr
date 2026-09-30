package dev.kampr.shared.net

import java.net.InetAddress
import java.net.Socket
import javax.net.SocketFactory

// Every socket the app dials carries keystrokes, a few bytes at a time, and OkHttp never turns
// Nagle off: a keystroke written while the last was unacknowledged waited for the ACK. TLS is
// layered over the socket this returns, so the flag holds for wss as well.
internal object NoDelaySockets : SocketFactory() {
    private val platform = getDefault()

    private fun Socket.eager() = apply { tcpNoDelay = true }

    override fun createSocket(): Socket = platform.createSocket().eager()

    override fun createSocket(host: String, port: Int): Socket = platform.createSocket(host, port).eager()

    override fun createSocket(host: String, port: Int, local: InetAddress, localPort: Int): Socket =
        platform.createSocket(host, port, local, localPort).eager()

    override fun createSocket(host: InetAddress, port: Int): Socket = platform.createSocket(host, port).eager()

    override fun createSocket(host: InetAddress, port: Int, local: InetAddress, localPort: Int): Socket =
        platform.createSocket(host, port, local, localPort).eager()
}
