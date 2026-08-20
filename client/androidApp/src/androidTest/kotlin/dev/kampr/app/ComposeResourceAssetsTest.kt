package dev.kampr.app

import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertTrue
import org.junit.Test

private const val FONT_DIR = "composeResources/dev.kampr.shared.res/font"

// Probe #64: CMP 1.11.1 ships no Android assets for a KMP-library target, and the failure is
// silent — Compose falls back to system sans. This asserts against the installed APK.
class ComposeResourceAssetsTest {
    private val assets = InstrumentationRegistry.getInstrumentation().targetContext.assets

    @Test
    fun everyFontIsReadableFromTheInstalledApk() {
        val fonts = assets.list(FONT_DIR).orEmpty()
        assertTrue("no fonts under $FONT_DIR", fonts.size >= 21)
        for (font in fonts) {
            val bytes = assets.open("$FONT_DIR/$font").use { it.readBytes() }
            assertTrue("$font is empty", bytes.size > 1024)
            assertTrue(
                "$font is not a TrueType/OpenType file",
                bytes.copyOfRange(0, 4).contentEquals(byteArrayOf(0, 1, 0, 0)) ||
                    bytes.copyOfRange(0, 4).contentEquals("OTTO".toByteArray()) ||
                    bytes.copyOfRange(0, 4).contentEquals("true".toByteArray()),
            )
        }
    }
}
