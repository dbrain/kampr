package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.text.BasicText
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.platform.Font
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.sp
import dev.kampr.shared.theme.FamilyId
import dev.kampr.shared.theme.GlyphGaps
import dev.kampr.shared.theme.gapsOfForTest
import dev.kampr.shared.theme.withGlyphFallback
import org.jetbrains.skia.Bitmap
import org.jetbrains.skia.ColorAlphaType
import org.jetbrains.skia.ImageInfo
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertNull
import kotlin.test.assertSame
import kotlin.test.assertTrue

private val FONTS = File("src/commonMain/composeResources/font")

private fun face(name: String, weight: FontWeight = FontWeight.W400) =
    Font(name, File(FONTS, "$name.ttf").readBytes(), weight)

private val UI = FontFamily(face("manrope_400"))
private val TERMINAL = FontFamily(face("terminalmono_regular"))

private fun ink(text: AnnotatedString, family: FontFamily): Int {
    val scene = ImageComposeScene(width = 200, height = 120, density = Density(1f)) {
        Box(Modifier.fillMaxSize()) {
            BasicText(
                text,
                style = TextStyle(fontFamily = family, fontSize = 56.sp, color = Color.White),
            )
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

// The other half of the report. `terminalmono` is rebuilt to carry every symbol an agent draws,
// and the conversation does not use it: its prose is Manrope or Archivo, which carry 678 and 653
// codepoints. Measured over the *prose* of this machine's transcripts — the text blocks the
// conversation actually renders — **91 distinct codepoints** neither the UI face nor the mono face
// can draw, led by `✅` at 73 occurrences across 9 files and `●` at 52 across 11 (#420).
//
// Nothing can be done with a family: a second face in one supplies nothing (#416). So the text
// names the terminal family over exactly the characters its own face cannot draw.
class GlyphFallbackTest {
    private val manrope = gapsOfForTest(FamilyId.Manrope)

    @Test
    fun aSymbolTheProseFaceCannotDrawIsAimedAtTheOneThatCan() {
        assertTrue(manrope.holds(0x2705), "✅ is the most common missing codepoint in the corpus")
        assertTrue(manrope.holds(0x25CF), "● opens every message Claude writes")
        assertTrue(manrope.holds(0x1F3A7), "🎧 is the symbol the report was about")
    }

    // The rule that keeps this from being a regression: it may turn tofu into a glyph and may
    // never move a glyph that already draws. Manrope has these and the terminal face does not.
    @Test
    fun aSymbolTheProseFaceCanAlreadyDrawIsLeftExactlyWhereItIs() {
        for (kept in listOf(0x20A9 /* ₩ */, 0x20B9 /* ₹ */, 0x21B5 /* ↵ */)) {
            assertTrue(!manrope.holds(kept), "U+%04X is in Manrope and must not be re-aimed".format(kept))
        }
        assertTrue(!manrope.holds('e'.code) && !manrope.holds(0x2019), "ordinary prose is untouched")
    }

    // A mark drawn from a different face than the letter it sits on lands in the wrong place, and
    // a zero-width joiner given a family of its own splits an emoji sequence down the middle.
    @Test
    fun aCombiningMarkOrAJoinerIsNeverGivenAFamilyOfItsOwn() {
        for (glue in listOf(0x200D, 0xFE0F, 0xFE0E, 0x0301, 0xE0061, 0x00)) {
            assertTrue(!manrope.holds(glue), "U+%04X must stay with what it joins".format(glue))
        }
    }

    // The common path costs nothing: an ASCII string comes back as null and the caller draws the
    // `String` it already had.
    @Test
    fun proseWithNothingToReAimIsHandedBackUntouched() {
        assertNull("an ordinary sentence, with punctuation — and a dash.".withGlyphFallback(manrope, TERMINAL))
        val already = AnnotatedString("plain")
        assertSame(already, already.withGlyphFallback(manrope, TERMINAL))
        assertNull("anything".withGlyphFallback(GlyphGaps.none, TERMINAL))
    }

    // Astral symbols are surrogate pairs, and the span has to cover both halves or the shaper is
    // handed half a codepoint.
    @Test
    fun anAstralSymbolIsSpannedAcrossBothHalvesOfItsPair() {
        val routed = "a 🎧 b".withGlyphFallback(manrope, TERMINAL)!!
        val spans = routed.spanStyles.filter { it.item.fontFamily == TERMINAL }
        assertEquals(1, spans.size)
        assertEquals("🎧", routed.text.substring(spans[0].start, spans[0].end))
    }

    // A row of box-drawing characters is one span, not six: a span per character breaks the shaper
    // into six runs, and #59 measured that shaping is the whole cost of a frame.
    @Test
    fun aRunOfSymbolsIsOneSpanRatherThanOnePerCharacter() {
        val routed = "tree ⎿⎯⎯⎯ end".withGlyphFallback(manrope, TERMINAL)!!
        val spans = routed.spanStyles.filter { it.item.fontFamily == TERMINAL }
        assertEquals(1, spans.size, "${spans.size} spans for one run")
        assertEquals("⎿⎯⎯⎯", routed.text.substring(spans[0].start, spans[0].end))
    }

    // Whatever the markdown builder already put on the text — code, links, emphasis — survives.
    @Test
    fun theSpansTheTextAlreadyCarriedAreKept() {
        val source = buildAnnotatedString {
            append("see ")
            withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append("this ✅ here") }
        }
        val routed = source.withGlyphFallback(manrope, TERMINAL)
        assertEquals(source.text, routed.text)
        assertTrue(routed.spanStyles.any { it.item.fontWeight == FontWeight.Bold }, "the bold span was dropped")
        assertTrue(routed.spanStyles.any { it.item.fontFamily == TERMINAL })
    }

    // And the whole point, rendered: the same string, drawn in the prose face, with and without the
    // routing. Asserted against the *terminal* face's own rendering rather than against emptiness,
    // because a desktop JVM may still find a glyph in a system font and a browser has nothing.
    @Test
    fun theSymbolIsDrawnFromTheTerminalFaceOnceItIsRouted() {
        val plain = AnnotatedString("✅")
        val routed = "✅".withGlyphFallback(manrope, TERMINAL)!!

        val asProse = ink(plain, UI)
        val asRouted = ink(routed, UI)
        val asTerminal = ink(plain, TERMINAL)

        assertTrue(asTerminal > 0, "the terminal face does not draw it, so this proves nothing")
        assertNotEquals(asProse, asTerminal, "the two faces draw it identically, so this proves nothing")
        assertEquals(
            asTerminal,
            asRouted,
            "routing did not change what was drawn: the prose face is still being asked for a " +
                "glyph it does not have, which in a browser is tofu",
        )
    }
}
