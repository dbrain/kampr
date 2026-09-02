package dev.kampr.shared.theme

import androidx.compose.runtime.Immutable
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily

// Which codepoints one chosen face cannot draw and the terminal face can, as the flat ascending
// `[first, last]` pairs `tools/terminalmono.py --gaps` generates.
//
// **Why this exists at all.** A `FontFamily` of loaded fonts draws everything from its first font
// and a second face in it supplies nothing, either way round (#416). The terminal face is rebuilt
// to carry every symbol an agent draws; the UI faces are stock and cannot be. So the only way a
// paragraph of prose can show `✅` or `●` is for the text itself to name a different family over
// exactly those characters — measured at 91 distinct codepoints in the prose of this machine's own
// transcripts, led by `✅` at 73 occurrences across 9 files (#420).
@Immutable
class GlyphGaps internal constructor(private val ranges: IntArray) {
    fun holds(codePoint: Int): Boolean {
        // ASCII is in every face by construction, and it is almost all of every string. One
        // comparison per character is the whole cost of the common path.
        if (codePoint < 0xA0) return false
        var low = 0
        var high = ranges.size / 2 - 1
        while (low <= high) {
            val mid = (low + high) / 2
            when {
                codePoint < ranges[mid * 2] -> high = mid - 1
                codePoint > ranges[mid * 2 + 1] -> low = mid + 1
                else -> return true
            }
        }
        return false
    }

    companion object {
        val none = GlyphGaps(IntArray(0))
    }
}

internal fun gapsOf(id: FamilyId) = GlyphGaps(gapsFor(id))

// The same table, reachable from a test that has to assert what it holds. `internal` does not
// cross the module boundary the jvmTest source set sits on.
fun gapsOfForTest(id: FamilyId): GlyphGaps = gapsOf(id)

// The text with its unsupported symbols re-aimed at the terminal face, or **the same instance back**
// when there is nothing to re-aim — which is every ASCII string in the app, so the common path
// allocates nothing and compares nothing beyond a byte per character.
//
// Runs are coalesced: a row of six box-drawing characters is one span, not six, because a span per
// character would break the shaper into six runs and #59 measured what that costs.
fun AnnotatedString.withGlyphFallback(gaps: GlyphGaps, terminal: FontFamily): AnnotatedString {
    if (!needsFallback(text, gaps)) return this
    val source = this
    return buildAnnotatedString {
        append(source)
        var at = 0
        while (at < text.length) {
            val start = at
            val run = runOf(text, at, gaps)
            at = run
            if (run == start) {
                at = start + charCount(text, start)
                continue
            }
            addStyle(SpanStyle(fontFamily = terminal), start, run)
        }
    }
}

fun String.withGlyphFallback(gaps: GlyphGaps, terminal: FontFamily): AnnotatedString? {
    if (!needsFallback(this, gaps)) return null
    return AnnotatedString(this).withGlyphFallback(gaps, terminal)
}

private fun needsFallback(text: String, gaps: GlyphGaps): Boolean {
    var at = 0
    while (at < text.length) {
        val code = text.codePointAtIndex(at)
        if (gaps.holds(code)) return true
        at += charCount(text, at)
    }
    return false
}

// The end of the run of consecutive routable characters starting at `from`, or `from` when the
// character there is not one.
private fun runOf(text: String, from: Int, gaps: GlyphGaps): Int {
    var at = from
    while (at < text.length && gaps.holds(text.codePointAtIndex(at))) {
        at += charCount(text, at)
    }
    return at
}

private fun charCount(text: String, at: Int): Int =
    if (text[at].isHighSurrogate() && at + 1 < text.length && text[at + 1].isLowSurrogate()) 2 else 1

// An astral symbol is a surrogate pair, and half of one is not a codepoint any face has an opinion
// about — every emoji this exists for lives above U+FFFF.
private fun String.codePointAtIndex(at: Int): Int {
    val high = this[at]
    if (high.isHighSurrogate() && at + 1 < length && this[at + 1].isLowSurrogate()) {
        return 0x10000 + ((high.code - 0xD800) shl 10) + (this[at + 1].code - 0xDC00)
    }
    return high.code
}
