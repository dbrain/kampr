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

fun inlineMarkdown(source: String, styles: InlineStyles): AnnotatedString = buildAnnotatedString {
    Inline(source, styles, this).run()
}

private class Inline(
    private val src: String,
    private val styles: InlineStyles,
    private val out: AnnotatedString.Builder,
    // Off inside the label of a written link, which is already a target: scanning it again would
    // stack a second annotation over the same characters and aim it somewhere else.
    private val linkify: Boolean = true,
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
                c == '!' && src.startsWith("![", at) -> { flush(); if (!image()) pending.append(src[at++]) }
                c == '[' -> { flush(); if (!link()) pending.append(src[at++]) }
                c == '<' -> { flush(); if (!autolink()) pending.append(src[at++]) }
                c == '~' && src.startsWith("~~", at) -> {
                    flush(); if (!wrap("~~", SpanStyle(textDecoration = TextDecoration.LineThrough))) pending.append(src[at++])
                }
                c == '*' || c == '_' -> { flush(); if (!emphasis(c)) pending.append(src[at++]) }
                c == 'h' || c == 'H' || c == 'm' || c == 'M' -> {
                    val url = if (linkify) urlAt(src, at) else null
                    if (url == null) { pending.append(c); at++ } else { flush(); target(url); at += url.length }
                }
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

    // Kampr carries no image bytes, so a picture is named where it stood. Left to the link rule
    // this rendered as a stray `!` beside link-styled alt text, and as nothing but `!` when the
    // alt was empty. The node names a transcript image the same way.
    private fun image(): Boolean {
        val close = matching(at + 1, '[', ']') ?: return false
        if (close + 1 >= src.length || src[close + 1] != '(') return false
        val end = matching(close + 1, '(', ')') ?: return false
        val alt = src.substring(at + 2, close).trim()
        out.append(if (alt.isEmpty()) "[image]" else "[image · $alt]")
        at = end + 1
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
        Inline(label, styles, out, linkify = false).run()
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
        target(url)
        at = close + 1
        return true
    }

    private fun target(url: String) {
        val start = out.length
        out.append(url)
        out.addStyle(styles.link, start, out.length)
        out.addLink(LinkAnnotation.Url(url, TextLinkStyles(styles.link)), start, out.length)
    }

    private fun emphasis(marker: Char): Boolean {
        if (!opens(marker, at)) return false
        val double = src.startsWith("$marker$marker", at)
        val token = if (double) "$marker$marker" else "$marker"
        val style = if (double) SpanStyle(fontWeight = FontWeight.Bold) else SpanStyle(fontStyle = FontStyle.Italic)
        return wrap(token, style) { closes(marker, it) }
    }

    // CommonMark 6.2, and the reason the whole run is measured rather than the one character under
    // the cursor: a delimiter is emphasis only where a word begins or ends. `_` is stricter than
    // `*` by design, because identifiers are made of it — `SC_4.00bpw_H5` and
    // `kampr_core::pane_registry` are words with underscores in them, not italics.
    private fun runOf(marker: Char, from: Int): Int {
        var run = 0
        while (from + run < src.length && src[from + run] == marker) run++
        return run
    }

    private fun flanking(start: Int, run: Int): Pair<Boolean, Boolean> {
        val before = src.getOrNull(start - 1)
        val after = src.getOrNull(start + run)
        val left = after != null && !after.isWhitespace() &&
            (!punctuation(after) || before == null || before.isWhitespace() || punctuation(before))
        val right = before != null && !before.isWhitespace() &&
            (!punctuation(before) || after == null || after.isWhitespace() || punctuation(after))
        return left to right
    }

    private fun opens(marker: Char, start: Int): Boolean {
        val (left, right) = flanking(start, runOf(marker, start))
        if (marker != '_') return left
        val before = src.getOrNull(start - 1)
        return left && (!right || (before != null && punctuation(before)))
    }

    private fun closes(marker: Char, start: Int): Boolean {
        val run = runOf(marker, start)
        val (left, right) = flanking(start, run)
        if (marker != '_') return right
        val after = src.getOrNull(start + run)
        return right && (!left || (after != null && punctuation(after)))
    }

    private fun punctuation(c: Char) = !c.isLetterOrDigit() && !c.isWhitespace()

    private fun wrap(token: String, style: SpanStyle, closes: (Int) -> Boolean = { true }): Boolean {
        var probe = at + token.length
        while (probe < src.length) {
            if (src[probe] == '\\') { probe += 2; continue }
            if (src.startsWith(token, probe) && closes(probe)) break
            probe++
        }
        if (probe >= src.length) return false
        val body = src.substring(at + token.length, probe)
        if (body.isEmpty()) return false
        val start = out.length
        Inline(body, styles, out, linkify).run()
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
