package dev.kampr.shared

import org.jetbrains.skia.Data
import org.jetbrains.skia.Font
import org.jetbrains.skia.FontMgr
import org.jetbrains.skia.Typeface
import java.io.File
import kotlin.test.Test
import kotlin.test.assertTrue

// Probe #270. The face is asked through Skia rather than by parsing its cmap, because Skia is what
// actually resolves a codepoint in the browser, and the browser is where this bites: there is no
// system font behind it and a FontFamily of loaded fonts resolves to exactly one typeface, so a
// codepoint terminalmono lacks is tofu and nothing else can supply it.
private val FACES = listOf(
    "terminalmono_regular",
    "terminalmono_bold",
    "terminalmono_italic",
    "terminalmono_bolditalic",
)

private class Face(val name: String, root: File = File("src/commonMain/composeResources/font")) {
    private val bytes = File(root, "$name.ttf").readBytes()
    val typeface: Typeface = FontMgr.default.makeFromData(Data.makeFromBytes(bytes))!!
    val font = Font(typeface, EM)

    fun draws(codePoint: Int) = typeface.getUTF32Glyph(codePoint) != 0.toShort()

    // Asked of Skia rather than of a cmap parser, for the reason above: Skia is what resolves a
    // codepoint in the browser. A face's own coverage is walked over the planes it can reach.
    fun codePoints(): List<Int> =
        ((0x20..0x2FFFF) + (0xE0000..0xE01FF)).filter { draws(it) }

    fun advanceOf(codePoint: Int): Float {
        val glyph = typeface.getUTF32Glyph(codePoint)
        return font.getWidths(shortArrayOf(glyph)).single()
    }

    companion object {
        const val EM = 100f
    }
}

private class Required(val codePoint: Int, val name: String)

private fun required(): List<Required> {
    val text = File("src/jvmTest/resources/agent-glyphs.txt").readText()
    return text.lineSequence()
        .filter { it.isNotBlank() && !it.startsWith("#") }
        .map { line ->
            val code = line.substringBefore(" ").removePrefix("U+")
            Required(code.toInt(16), line.substringAfter("  ").trim())
        }
        .toList()
}

private fun describe(items: List<Required>) =
    items.joinToString("\n") { "    U+%04X %s".format(it.codePoint, it.name) }

class TerminalFontCoverageTest {

    // The defect this is named for: U+23BF and U+23AF, the two characters an agent draws its
    // tool-result tree with, were absent from all four faces and rendered as tofu in the browser.
    // Asserting those two would have been worthless a week later, so the set is the assertion:
    // every codepoint measured on a real agent screen, in every face, or this goes red.
    @Test
    fun everyGlyphARealAgentScreenDrawsIsInAllFourTerminalFaces() {
        val wanted = required()
        assertTrue(wanted.size > 100, "the glyph set did not load: ${wanted.size} rows")
        val complaints = FACES.mapNotNull { name ->
            val face = Face(name)
            val absent = wanted.filterNot { face.draws(it.codePoint) }
            if (absent.isEmpty()) null else "$name is missing ${absent.size}:\n${describe(absent)}"
        }
        assertTrue(complaints.isEmpty(), complaints.joinToString("\n\n"))
    }

    // The invariant the face is built on, asserted rather than assumed: a symbol cut in from
    // another family must not widen the cell it lands in. Every glyph is measured against the
    // face's own digit zero, so this stays true whatever the em is scaled to.
    @Test
    fun noGlyphARealAgentScreenDrawsIsWiderThanOneCell() {
        val wanted = required()
        val complaints = FACES.mapNotNull { name ->
            val face = Face(name)
            val cell = face.advanceOf('0'.code)
            val wrong = wanted.filter { face.advanceOf(it.codePoint) != cell }
            if (wrong.isEmpty()) {
                null
            } else {
                "$name draws ${wrong.size} at an advance that is not the $cell of a cell:\n" +
                    wrong.joinToString("\n") {
                        "    U+%04X %s at %s".format(it.codePoint, it.name, face.advanceOf(it.codePoint))
                    }
            }
        }
        assertTrue(complaints.isEmpty(), complaints.joinToString("\n\n"))
    }

    // Probe #417. The assertion that closes the class rather than one report of it. A census names what a
    // screen has already drawn; a harness prints whatever a person or a tool picked, and the
    // report this was written for was a headphone an artifact chose as its own icon. So the
    // contract is the donor: every codepoint the vendored monochrome Noto Emoji face draws is in
    // all four terminal faces, and `tools/terminalmono.py --write` is what keeps it true.
    @Test
    fun everyEmojiTheDonorDrawsIsInAllFourTerminalFaces() {
        val donor = Face("../../tools/donors/NotoEmoji-Regular", root = File("."))
        val wanted = donor.codePoints()
        assertTrue(wanted.size > 1000, "the emoji donor did not load: ${wanted.size} codepoints")
        val complaints = FACES.mapNotNull { name ->
            val face = Face(name)
            val absent = wanted.filterNot { face.draws(it) }
            if (absent.isEmpty()) {
                null
            } else {
                "$name is missing ${absent.size} of the donor's emoji, first few:\n" +
                    absent.take(8).joinToString("\n") { "    U+%04X %s".format(it, String(Character.toChars(it))) }
            }
        }
        assertTrue(complaints.isEmpty(), complaints.joinToString("\n\n"))
    }

    // And none of them widened a cell on the way in. Asserted over the donor's whole set rather
    // than over the census, because that is the set that was added.
    @Test
    fun noEmojiCutInWidensTheCellItLandsIn() {
        val wanted = Face("../../tools/donors/NotoEmoji-Regular", root = File(".")).codePoints()
        val complaints = FACES.mapNotNull { name ->
            val face = Face(name)
            val cell = face.advanceOf('0'.code)
            val wrong = wanted.filter { face.advanceOf(it) != cell }
            if (wrong.isEmpty()) null else "$name draws ${wrong.size} emoji at an advance that is not $cell"
        }
        assertTrue(complaints.isEmpty(), complaints.joinToString("\n"))
    }

    // The other half of "no symbol widens a cell or grows a line". A cut-in carrying its donor's
    // vertical metrics would grow every line in the pane, whether or not that donor's glyph is
    // ever drawn, so the four faces must agree with each other and with the base family.
    @Test
    fun noFaceGrowsTheLineTheOthersDraw() {
        val metrics = FACES.associateWith { name ->
            val m = Face(name).font.metrics
            listOf(m.ascent, m.descent, m.leading).map { it / Face.EM }
        }
        val distinct = metrics.values.distinct()
        assertTrue(
            distinct.size == 1,
            "the four faces do not share one line box, so a pane's row height depends on style:\n" +
                metrics.entries.joinToString("\n") { "    ${it.key} ${it.value}" },
        )
    }
}
