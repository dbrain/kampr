package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.text.BasicText
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.platform.Font
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.sp
import org.jetbrains.skia.Bitmap
import org.jetbrains.skia.ColorAlphaType
import org.jetbrains.skia.ImageInfo
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

private val FONTS = File("src/commonMain/composeResources/font")

private fun face(name: String) = Font(name, File(FONTS, "$name.ttf").readBytes(), FontWeight.W400)

// Lit subpixels rather than a control image: an antialiasing difference of a few pixels is not a
// different glyph, and #271 already learned that comparing images here fails on 212 of them.
private fun ink(family: FontFamily, text: String): Int {
    val scene = ImageComposeScene(width = 140, height = 140, density = Density(1f)) {
        Box(Modifier.fillMaxSize()) {
            BasicText(text, style = TextStyle(fontFamily = family, fontSize = 72.sp, color = Color.White))
        }
    }
    val image = scene.render()
    val info = ImageInfo.makeN32(image.width, image.height, ColorAlphaType.UNPREMUL)
    val bitmap = Bitmap().also { it.allocPixels(info) }
    check(image.readPixels(bitmap)) { "the scene rendered no pixels" }
    val bytes = bitmap.readPixels()!!
    scene.close()
    return (bytes.indices step 4).count { bytes[it].toInt() and 0xFF != 0 }
}

// The claim the whole terminal-face build rests on, asserted rather than repeated. `terminalmono`
// exists — 2 235 glyphs cut in from four donors by `tools/terminalmono.py` — only because a
// codepoint the face lacks cannot be supplied by anything else in the browser. If that were wrong,
// the answer would be one emoji font added beside the face and none of the merging.
//
// It is not wrong. Measured here on skiko (#416), and the direction is what makes it airtight: the family
// draws whatever its **first** font draws, both ways round, and the second font is never consulted
// for a glyph the first is missing.
class FontFamilyFallbackTest {
    private val mono = face("terminalmono_regular")
    private val ui = face("manrope_400")

    // U+29C9 is in `terminalmono` (cut in for probe #270) and not in Manrope. On this JVM Skia may
    // still find it in a system font, which is exactly why the assertion is against the *donor*
    // rendering and not against emptiness: what has to be shown is that pairing changes nothing.
    private val absentFromUi = "⧉"

    @Test
    fun aSecondFontInAFamilyDoesNotSupplyAGlyphTheFirstIsMissing() {
        val alone = ink(FontFamily(ui), absentFromUi)
        val paired = ink(FontFamily(ui, mono), absentFromUi)
        val donor = ink(FontFamily(mono), absentFromUi)

        assertTrue(donor > 0, "the donor face does not draw the probe glyph, so this proves nothing")
        assertNotEquals(alone, donor, "the two faces draw it identically, so this proves nothing")
        assertEquals(
            alone,
            paired,
            "adding a face that draws U+29C9 to the family changed what was drawn — skiko does " +
                "resolve past the first font, and terminalmono could be a much smaller face",
        )
    }

    // The other direction, so this cannot be read as "the wider face always wins": the first font
    // is drawn from for a letter the second one also has, and the second is never reached.
    @Test
    fun theFirstFontInAFamilyIsTheOneThatDrawsEverything() {
        val monoFirst = ink(FontFamily(mono, ui), "A")
        val uiFirst = ink(FontFamily(ui, mono), "A")
        assertNotEquals(monoFirst, uiFirst, "the two faces draw A identically, so this proves nothing")
        assertEquals(ink(FontFamily(mono), "A"), monoFirst)
        assertEquals(ink(FontFamily(ui), "A"), uiFirst)
    }
}
