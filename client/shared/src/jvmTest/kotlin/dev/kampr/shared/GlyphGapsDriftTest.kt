package dev.kampr.shared

import dev.kampr.shared.theme.FamilyId
import dev.kampr.shared.theme.gapsOfForTest
import org.jetbrains.skia.Data
import org.jetbrains.skia.FontMgr
import org.jetbrains.skia.Typeface
import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue

private val FONTS = File("src/commonMain/composeResources/font")

private fun coverage(prefix: String): (Int) -> Boolean {
    val faces = FONTS.listFiles { f: File -> f.name.startsWith("${prefix}_") && f.name.endsWith(".ttf") }
        .orEmpty()
        .map { FontMgr.default.makeFromData(Data.makeFromBytes(it.readBytes()))!! }
    check(faces.isNotEmpty()) { "no faces for $prefix" }
    return { cp -> faces.any { it.getUTF32Glyph(cp) != 0.toShort() } }
}

private val PREFIXES = mapOf(
    FamilyId.Manrope to "manrope",
    FamilyId.IbmPlexMono to "ibmplexmono",
    FamilyId.JetBrainsMono to "jetbrainsmono",
    FamilyId.InstrumentSans to "instrumentsans",
    FamilyId.Archivo to "archivo",
)

// `GlyphGaps.kt` is generated from the shipped `.ttf` files by `terminalmono.py --gaps`, and a
// generated file that nothing checks is a file that drifts. Rebuilding the faces without
// regenerating it would leave the new emoji unrouted and the report open again — silently, because
// tofu in a browser is not something any other test here can see.
//
// The faces are asked through Skia rather than by parsing a cmap, for the reason
// `TerminalFontCoverageTest` gives: Skia is what resolves a codepoint in the browser.
class GlyphGapsDriftTest {
    @Test
    fun everyGapTableStillDescribesTheFacesThatShip() {
        val terminal = coverage("terminalmono")
        val complaints = PREFIXES.mapNotNull { (id, prefix) ->
            val face = coverage(prefix)
            val table = gapsOfForTest(id)
            // Only the routable classes are in the table by design; a letter or a joiner is not,
            // and asserting over the whole plane would fail on exactly those.
            val wrong = (0x20..0x2FFFF).filter { cp ->
                routable(cp) && terminal(cp) && !face(cp) != table.holds(cp)
            }
            if (wrong.isEmpty()) {
                null
            } else {
                "$id: ${wrong.size} codepoints disagree, first few " +
                    wrong.take(8).joinToString(" ") { "U+%04X".format(it) }
            }
        }
        assertTrue(
            complaints.isEmpty(),
            "run `python3 tools/terminalmono.py --gaps`:\n" + complaints.joinToString("\n"),
        )
    }

    // The same classes the generator keeps: a standalone symbol, punctuation mark or symbolic
    // numeral, and nothing that has to stay attached to something else.
    private fun routable(cp: Int): Boolean = when (Character.getType(cp).toByte()) {
        Character.MATH_SYMBOL, Character.CURRENCY_SYMBOL, Character.MODIFIER_SYMBOL,
        Character.OTHER_SYMBOL, Character.DASH_PUNCTUATION, Character.START_PUNCTUATION,
        Character.END_PUNCTUATION, Character.INITIAL_QUOTE_PUNCTUATION,
        Character.FINAL_QUOTE_PUNCTUATION, Character.OTHER_PUNCTUATION,
        Character.CONNECTOR_PUNCTUATION, Character.OTHER_NUMBER,
        -> true
        else -> false
    }
}
