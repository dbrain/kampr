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

private class Face(val name: String) {
    private val bytes = File("src/commonMain/composeResources/font/$name.ttf").readBytes()
    val typeface: Typeface = FontMgr.default.makeFromData(Data.makeFromBytes(bytes))!!
    val font = Font(typeface, EM)

    fun draws(codePoint: Int) = typeface.getUTF32Glyph(codePoint) != 0.toShort()

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
