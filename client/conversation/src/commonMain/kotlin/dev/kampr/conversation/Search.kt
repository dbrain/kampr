package dev.kampr.conversation

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn

fun blockText(block: Block): String = when (block) {
    is Block.Md -> block.text
    is Block.Code -> block.text
    is Block.Diff -> listOfNotNull(block.path, block.text).joinToString("\n")
    is Block.Tool -> listOfNotNull(block.name, block.summary).joinToString(" ")
    is Block.Unknown -> ""
}

fun turnText(turn: Turn): String = turn.blocks.joinToString("\n", transform = ::blockText)

fun searchHits(turns: List<Turn>, query: String): List<Int> {
    if (query.length < 2) return emptyList()
    return turns.indices.filter { turnText(turns[it]).contains(query, ignoreCase = true) }
}

fun matchRanges(text: String, query: String): List<IntRange> {
    if (query.length < 2) return emptyList()
    val out = mutableListOf<IntRange>()
    var at = text.indexOf(query, 0, ignoreCase = true)
    while (at >= 0) {
        out += at until (at + query.length)
        at = text.indexOf(query, at + query.length, ignoreCase = true)
    }
    return out
}

fun AnnotatedString.markMatches(query: String, ground: Color): AnnotatedString {
    val ranges = matchRanges(text, query)
    if (ranges.isEmpty()) return this
    return AnnotatedString.Builder(this).apply {
        for (range in ranges) addStyle(SpanStyle(background = ground), range.first, range.last + 1)
    }.toAnnotatedString()
}
