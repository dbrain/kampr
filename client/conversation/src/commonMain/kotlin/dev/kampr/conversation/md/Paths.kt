package dev.kampr.conversation.md

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import dev.kampr.shared.model.asPath

// A path rides a URL annotation under a scheme no browser knows, so every surface that already
// hands its links to `LocalUriHandler` hands paths to the same place, and the conversation's own
// handler is the one thing that has to tell them apart.
private const val PATH_SCHEME = "kampr-path:"

private const val SENTENCE_TAIL = ".,;:!?)'\""

fun pathLink(path: String): String = PATH_SCHEME + path

fun pathOfLink(uri: String): String? = uri.takeIf { it.startsWith(PATH_SCHEME) }?.removePrefix(PATH_SCHEME)?.ifEmpty { null }

private fun startsWord(src: CharSequence, at: Int): Boolean = at == 0 || src[at - 1].isWhitespace() || src[at - 1] in "([\"'"

// The length of the path starting at `at` and the path itself, or null. The whole whitespace-delimited token is taken and
// then the sentence's own punctuation is given back, so `see ./a.sh.` ends before the full stop.
internal fun pathAt(src: CharSequence, at: Int): Pair<Int, String>? {
    if (!startsWord(src, at)) return null
    val first = src[at]
    if (!(first.isLetterOrDigit() || first == '/' || first == '~' || first == '.' || first == '_')) return null
    var end = at
    while (end < src.length && !src[end].isWhitespace() && src[end] != '`') end++
    while (end > at && src[end - 1] in SENTENCE_TAIL) end--
    if (end <= at) return null
    val token = src.substring(at, end)
    val path = asPath(token, bare = true) ?: return null
    return token.length to path
}

fun AnnotatedString.markPaths(style: SpanStyle): AnnotatedString {
    val spans = mutableListOf<Triple<Int, Int, String>>()
    val taken = getLinkAnnotations(0, length)
    var at = 0
    while (at < text.length) {
        val found = pathAt(text, at)
        if (found == null || taken.any { at in it.start until it.end }) {
            at++
            continue
        }
        spans += Triple(at, at + found.first, found.second)
        at += found.first
    }
    if (spans.isEmpty()) return this
    return AnnotatedString.Builder(this).apply {
        for ((start, end, path) in spans) {
            addStyle(style, start, end)
            addLink(LinkAnnotation.Url(pathLink(path), TextLinkStyles(style)), start, end)
        }
    }.toAnnotatedString()
}
