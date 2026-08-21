package dev.kampr.shared

import dev.kampr.shared.util.joinLink
import dev.kampr.shared.util.qrEncode
import java.awt.image.BufferedImage
import java.io.File
import javax.imageio.ImageIO
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.fail

private val OUT = File("build/qr")

private const val SCALE = 6
private const val QUIET = 4

// Asserting on the bitmap this encoder produced would only prove it agrees with itself. zbar is a
// decoder nothing here wrote, so what it reads back is what a phone camera would read.
private fun decode(text: String, name: String): String {
    val code = assertNotNull(qrEncode(text), "nothing encoded for $text")
    val side = (code.size + QUIET * 2) * SCALE
    val image = BufferedImage(side, side, BufferedImage.TYPE_INT_RGB)
    val g = image.createGraphics()
    g.color = java.awt.Color.WHITE
    g.fillRect(0, 0, side, side)
    g.color = java.awt.Color.BLACK
    for (y in 0 until code.size) {
        for (x in 0 until code.size) {
            if (code.dark(x, y)) g.fillRect((x + QUIET) * SCALE, (y + QUIET) * SCALE, SCALE, SCALE)
        }
    }
    g.dispose()
    OUT.mkdirs()
    val file = File(OUT, "$name.png")
    ImageIO.write(image, "png", file)

    val process = ProcessBuilder("zbarimg", "--raw", "-q", file.absolutePath)
        .redirectErrorStream(true)
        .start()
    val read = process.inputStream.bufferedReader().readText()
    process.waitFor()
    return read.trim()
}

private val ZBAR: Boolean = runCatching {
    ProcessBuilder("zbarimg", "--version").start().waitFor() == 0
}.getOrDefault(false)

// A silent skip is how a suite comes to have never run once. This one says so on stderr, and
// KAMPR_QR_DECODE turns the skip into a failure the way KAMPR_LIVE does for LiveNodeTest.
private val REQUIRED: Boolean = System.getenv("KAMPR_QR_DECODE") != null

class QrDecodeTest {
    private fun usable(): Boolean {
        if (ZBAR) return true
        val why = "QrDecodeTest SKIPPED — zbarimg is not installed, no symbol was decoded"
        if (REQUIRED) fail("$why, and KAMPR_QR_DECODE demanded it")
        System.err.println("\n${"!".repeat(78)}\n  $why\n${"!".repeat(78)}\n")
        return false
    }

    private fun roundTrip(text: String, name: String) {
        if (!usable()) return
        assertEquals(text, decode(text, name), "zbar read something else back")
    }

    // The two shapes a real install produces: a LAN address on first light, and the hostname
    // behind a reverse proxy once the ladder has been climbed.
    @Test
    fun aPairingLinkSurvivesAnIndependentDecoder() {
        roundTrip(joinLink("http://192.168.1.24:8790", "K7QF2M"), "lan-with-code")
        roundTrip(joinLink("https://kampr.example.com", "K7QF2M"), "proxied-with-code")
        roundTrip(joinLink("http://192.168.1.24:8790", null), "lan-bare")
    }

    // Version selection is the part that goes wrong silently: a wrong capacity table produces a
    // symbol that scans at one length and not at the next.
    @Test
    fun everyLengthUpToTheCapacityStillScans() {
        // 14 is the last byte a version 1 symbol holds and 213 the last a version 10 holds, so
        // this walks every capacity step the encoder can choose between.
        for (n in listOf(
            1, 14, 15, 26, 27, 42, 43, 62, 63, 84, 85, 106, 107, 122, 123, 152, 153, 180, 181, 213,
        )) {
            roundTrip("k.example/" .repeat(30).take(n), "len-$n")
        }
    }

    @Test
    fun somethingTooLongIsRefusedRatherThanTruncated() {
        assertNull(qrEncode("x".repeat(400)))
    }

    @Test
    fun theLinkIsTheOriginPlusAFragmentThatNeverReachesTheNode() {
        assertEquals("http://192.168.1.24:8790", joinLink("http://192.168.1.24:8790", null))
        assertEquals("http://192.168.1.24:8790#pair=ABC", joinLink("http://192.168.1.24:8790/", "ABC"))
        assertEquals("https://kampr.example.com#pair=ABC", joinLink("https://kampr.example.com", "ABC"))
    }
}
