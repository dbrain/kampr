package dev.kampr.conversation.syntax

data class Span(val start: Int, val end: Int, val token: Token)

fun scan(code: String, spec: LangSpec): List<Span> {
    val out = mutableListOf<Span>()
    var at = 0

    fun emit(from: Int, to: Int, token: Token) {
        if (to > from && token != Token.Plain) out += Span(from, to, token)
    }

    while (at < code.length) {
        val c = code[at]

        val line = spec.lineComment.firstOrNull { code.startsWith(it, at) }
        if (line != null) {
            val end = code.indexOf('\n', at).let { if (it < 0) code.length else it }
            emit(at, end, Token.Comment)
            at = end
            continue
        }

        val block = spec.blockComment
        if (block != null && code.startsWith(block.first, at)) {
            val close = code.indexOf(block.second, at + block.first.length)
            val end = if (close < 0) code.length else close + block.second.length
            emit(at, end, Token.Comment)
            at = end
            continue
        }

        if (c in spec.quotes && spec.quotes.isNotEmpty()) {
            var probe = at + 1
            while (probe < code.length) {
                if (code[probe] == '\\') { probe += 2; continue }
                if (code[probe] == c || code[probe] == '\n') break
                probe++
            }
            val end = if (probe < code.length && code[probe] == c) probe + 1 else probe
            emit(at, end, Token.Text)
            at = end
            continue
        }

        if (spec.meta != null && c == spec.meta) {
            var probe = at + 1
            if (probe < code.length && code[probe] == '[') probe++
            while (probe < code.length && (code[probe].isLetterOrDigit() || code[probe] == '_' || code[probe] == '!')) probe++
            emit(at, probe, Token.Meta)
            at = probe
            continue
        }

        if (c.isDigit() && (at == 0 || !isWordChar(code[at - 1]))) {
            var probe = at
            while (probe < code.length && (code[probe].isLetterOrDigit() || code[probe] == '.' || code[probe] == '_')) probe++
            emit(at, probe, Token.Number)
            at = probe
            continue
        }

        if (isWordStart(c)) {
            var probe = at
            while (probe < code.length && isWordChar(code[probe])) probe++
            val word = code.substring(at, probe)
            val next = code.getOrNull(probe.let { var p = it; while (p < code.length && code[p] == ' ') p++; p })
            val token = when {
                word in spec.keywords -> Token.Keyword
                word in spec.constants -> Token.Number
                next == '(' -> Token.Call
                else -> Token.Plain
            }
            emit(at, probe, token)
            at = probe
            continue
        }

        if (!c.isWhitespace() && !isWordChar(c)) {
            emit(at, at + 1, Token.Punct)
        }
        at++
    }
    return out
}

private fun isWordStart(c: Char) = c.isLetter() || c == '_'

private fun isWordChar(c: Char) = c.isLetterOrDigit() || c == '_'
