package dev.kampr.shared

import dev.kampr.shared.net.decodeQrLuminance
import dev.kampr.shared.util.joinLink
import dev.kampr.shared.util.qrEncode
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

// The scanner reads what the desktop draws. Both halves are in this repository and neither is the
// other's mirror: `qrEncode` is Kampr's own encoder, `decodeQrLuminance` is zxing. What this
// asserts is that the picture on one screen survives the library on the other phone.
//
// It runs on the host JVM rather than a device because nothing in the decode path is Android —
// a camera frame is a luminance plane, and this builds one.
private const val SCALE = 5
private const val QUIET = 4

// A camera frame, not a bitmap: 8-bit luminance, one byte per pixel, with a row stride that is
// usually wider than the image. Getting that padding wrong is the bug that only shows up on a
// phone, so it is a parameter here.
private fun frameOf(text: String, scale: Int = SCALE, padding: Int = 0, invert: Boolean = false): Triple<ByteArray, Int, Int> {
    val code = requireNotNull(qrEncode(text)) { "nothing encoded for $text" }
    val side = (code.size + QUIET * 2) * scale
    val stride = side + padding
    val luma = ByteArray(stride * side) { if (invert) 0 else 0xFF.toByte() }
    for (y in 0 until code.size) {
        for (x in 0 until code.size) {
            if (!code.dark(x, y)) continue
            for (dy in 0 until scale) {
                for (dx in 0 until scale) {
                    val px = (x + QUIET) * scale + dx
                    val py = (y + QUIET) * scale + dy
                    luma[py * stride + px] = if (invert) 0xFF.toByte() else 0
                }
            }
        }
    }
    return Triple(luma, stride, side)
}

class QrScanTest {
    @Test
    fun theQrTheDesktopDrawsIsTheQrThePhoneReads() {
        for (link in listOf(
            joinLink("http://192.168.1.24:8790", "K7QF2M"),
            joinLink("https://kampr.example.com", "K7QF2M"),
            joinLink("http://192.168.1.24:8790", null),
        )) {
            val (luma, stride, side) = frameOf(link)
            assertEquals(link, decodeQrLuminance(luma, stride, side, side), "did not read back $link")
        }
    }

    // A real `ImageProxy` hands over a plane whose `rowStride` is padded up to a hardware
    // alignment. Treating the stride as the width shears the image and nothing ever scans.
    @Test
    fun aPaddedCameraStrideIsNotPartOfThePicture() {
        val link = joinLink("https://kampr.example.com", "K7QF2M")
        for (padding in listOf(1, 16, 64)) {
            val (luma, stride, side) = frameOf(link, padding = padding)
            assertEquals(link, decodeQrLuminance(luma, stride, side, side), "stride padded by $padding")
        }
    }

    // The symbol is small in the frame when somebody holds the phone back a little.
    @Test
    fun aSmallSymbolInALargeFrameStillReads() {
        val link = joinLink("http://192.168.1.24:8790", "K7QF2M")
        val (luma, stride, side) = frameOf(link, scale = 2)
        assertEquals(link, decodeQrLuminance(luma, stride, side, side))
    }

    @Test
    fun aFrameWithNoSymbolInItIsNotAnError() {
        val blank = ByteArray(640 * 480) { 0xFF.toByte() }
        assertNull(decodeQrLuminance(blank, 640, 640, 480))
        assertNull(decodeQrLuminance(ByteArray(0), 0, 0, 0))
    }
}
