package dev.kampr.conversation.md

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles

// Agent output can carry fetched web content, so a URL is data, not an instruction: only schemes
// that can do nothing but navigate become live links, and anything else stays inert text.
private val SAFE_SCHEME = Regex("^(https?|mailto):", RegexOption.IGNORE_CASE)

private val BARE_SCHEMES = listOf("https://", "http://", "mailto:")

// GFM's set, and for the same reason: every one of these is legal inside a URL and is almost
// always the sentence's own punctuation when it is the last character before a space.
private const val SENTENCE_TAIL = ".,;:!?*_~'\""

internal fun safeUrl(raw: String): String? {
    val url = raw.trim()
    if (url.isEmpty() || url.any { it.isWhitespace() || it.code < 0x20 }) return null
    return url.takeIf { SAFE_SCHEME.containsMatchIn(it) }
}

// The one rule that turns a run of characters into a target, kept apart from the markdown scanner
// so plain text — tool output, a code block, a diff line — can ask the same question of the same
// characters, and so a second kind of target can be recognised beside this one.
internal fun urlAt(src: String, at: Int): String? {
    if (!wordStart(src, at)) return null
    val scheme = BARE_SCHEMES.firstOrNull { src.startsWith(it, at, ignoreCase = true) } ?: return null
    var end = at + scheme.length
    while (end < src.length && isUrlChar(src[end])) end++
    while (end > at + scheme.length && dropsFromTail(src, at, end)) end--
    if (end == at + scheme.length) return null
    return safeUrl(src.substring(at, end))
}

private fun findUrls(src: String): List<IntRange> {
    val out = mutableListOf<IntRange>()
    var at = 0
    while (at < src.length) {
        val url = urlAt(src, at)
        if (url == null) {
            at++
            continue
        }
        out += at until (at + url.length)
        at += url.length
    }
    return out
}

fun AnnotatedString.markUrls(style: SpanStyle): AnnotatedString {
    val spans = findUrls(text)
    if (spans.isEmpty()) return this
    return AnnotatedString.Builder(this).apply {
        for (span in spans) {
            val url = text.substring(span.first, span.last + 1)
            addStyle(style, span.first, span.last + 1)
            addLink(LinkAnnotation.Url(url, TextLinkStyles(style)), span.first, span.last + 1)
        }
    }.toAnnotatedString()
}

private fun wordStart(src: String, at: Int): Boolean =
    at == 0 || !src[at - 1].isLetterOrDigit()

private fun isUrlChar(c: Char): Boolean =
    !c.isWhitespace() && c.code >= 0x20 && c !in "<>\"`"

private fun dropsFromTail(src: String, from: Int, to: Int): Boolean {
    val last = src[to - 1]
    if (last in SENTENCE_TAIL) return true
    // A bracket the URL opened is its own; one it never opened closes something the sentence did.
    return last == ')' && depth(src, from, to) < 0
}

private fun depth(src: String, from: Int, to: Int): Int {
    var depth = 0
    for (index in from until to) {
        when (src[index]) {
            '(' -> depth++
            ')' -> depth--
        }
    }
    return depth
}
