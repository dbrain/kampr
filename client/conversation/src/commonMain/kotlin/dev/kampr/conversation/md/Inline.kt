package dev.kampr.conversation.md

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle

data class InlineStyles(val code: SpanStyle, val link: SpanStyle)

// Agent output can carry fetched web content, so a URL is data, not an instruction: only schemes
// that can do nothing but navigate become live links, and anything else stays inert text.
private val SAFE_SCHEME = Regex("^(https?|mailto):", RegexOption.IGNORE_CASE)

private fun safeUrl(raw: String): String? {
    val url = raw.trim()
    if (url.isEmpty() || url.any { it.isWhitespace() || it.code < 0x20 }) return null
    return url.takeIf { SAFE_SCHEME.containsMatchIn(it) }
}

fun inlineMarkdown(source: String, styles: InlineStyles): AnnotatedString = buildAnnotatedString {
    Inline(source, styles, this).run()
}

private class Inline(
    private val src: String,
    private val styles: InlineStyles,
    private val out: AnnotatedString.Builder,
) {
    private var at = 0

    fun run() {
        val pending = StringBuilder()
        fun flush() {
            if (pending.isNotEmpty()) {
                out.append(pending.toString())
                pending.setLength(0)
            }
        }
        while (at < src.length) {
            val c = src[at]
            when {
                c == '\\' && at + 1 < src.length && !src[at + 1].isLetterOrDigit() -> {
                    pending.append(src[at + 1]); at += 2
                }
                c == '`' -> { flush(); if (!code()) pending.append(src[at++]) }
                c == '[' -> { flush(); if (!link()) pending.append(src[at++]) }
                c == '<' -> { flush(); if (!autolink()) pending.append(src[at++]) }
                c == '~' && src.startsWith("~~", at) -> {
                    flush(); if (!wrap("~~", SpanStyle(textDecoration = TextDecoration.LineThrough))) pending.append(src[at++])
                }
                c == '*' || c == '_' -> { flush(); if (!emphasis(c)) pending.append(src[at++]) }
                else -> { pending.append(c); at++ }
            }
        }
        flush()
    }

    private fun code(): Boolean {
        var ticks = 0
        while (at + ticks < src.length && src[at + ticks] == '`') ticks++
        val fence = "`".repeat(ticks)
        val close = src.indexOf(fence, at + ticks)
        if (close < 0) return false
        val body = src.substring(at + ticks, close).trim(' ')
        out.withStyle(styles.code) { append(body) }
        at = close + ticks
        return true
    }

    private fun link(): Boolean {
        val close = matching(at, '[', ']') ?: return false
        if (close + 1 >= src.length || src[close + 1] != '(') return false
        val end = matching(close + 1, '(', ')') ?: return false
        val label = src.substring(at + 1, close)
        val target = src.substring(close + 2, end).substringBefore(' ')
        val url = safeUrl(target)
        val start = out.length
        Inline(label, styles, out).run()
        out.addStyle(styles.link, start, out.length)
        if (url != null) {
            out.addLink(LinkAnnotation.Url(url, TextLinkStyles(styles.link)), start, out.length)
        }
        at = end + 1
        return true
    }

    private fun autolink(): Boolean {
        val close = src.indexOf('>', at + 1)
        if (close < 0) return false
        val url = safeUrl(src.substring(at + 1, close)) ?: return false
        val start = out.length
        out.append(url)
        out.addStyle(styles.link, start, out.length)
        out.addLink(LinkAnnotation.Url(url, TextLinkStyles(styles.link)), start, out.length)
        at = close + 1
        return true
    }

    private fun emphasis(marker: Char): Boolean {
        val double = src.startsWith("$marker$marker", at)
        val token = if (double) "$marker$marker" else "$marker"
        val style = if (double) SpanStyle(fontWeight = FontWeight.Bold) else SpanStyle(fontStyle = FontStyle.Italic)
        return wrap(token, style)
    }

    private fun wrap(token: String, style: SpanStyle): Boolean {
        var probe = at + token.length
        while (probe < src.length) {
            if (src[probe] == '\\') { probe += 2; continue }
            if (src.startsWith(token, probe)) break
            probe++
        }
        if (probe >= src.length) return false
        val body = src.substring(at + token.length, probe)
        if (body.isEmpty()) return false
        val start = out.length
        Inline(body, styles, out).run()
        out.addStyle(style, start, out.length)
        at = probe + token.length
        return true
    }

    private fun matching(from: Int, open: Char, close: Char): Int? {
        var depth = 0
        var probe = from
        while (probe < src.length) {
            when {
                src[probe] == '\\' -> probe++
                src[probe] == open -> depth++
                src[probe] == close -> {
                    depth--
                    if (depth == 0) return probe
                }
            }
            probe++
        }
        return null
    }
}
