package dev.kampr.conversation

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn

fun blockText(block: Block): String = when (block) {
    // What the reader can see: a block carrying a header renders as a card, so a hit on the
    // marker it replaced would be a match the counter promises and nothing on screen shows.
    is Block.Md -> block.att?.let { listOfNotNull(it.name, it.mime).joinToString(" ") } ?: block.text
    is Block.Code -> block.text
    is Block.Diff -> listOfNotNull(block.path, block.text).joinToString("\n")
    is Block.Tool -> toolLabel(block, " ")
    // What the card shows of it, which is the agent's type and what it was asked — the turns
    // themselves are another conversation and are not on this screen to be found.
    is Block.Sub -> listOfNotNull(block.kind, block.title).joinToString(" ")
    is Block.Unknown -> ""
}

fun turnText(turn: Turn): String = turn.blocks.joinToString("\n", transform = ::blockText)

fun turnMatches(turn: Turn, query: String): Boolean =
    query.length >= 2 && turnText(turn).contains(query, ignoreCase = true)

// Rows, not turns: what a hit is worth is the list being able to aim at it, and a run of tool
// calls is one item of that list however many turns it holds. A run holding a match is never
// collapsed, so every index this returns is a row the reader can read once they arrive.
fun searchHits(rows: List<TranscriptRow>, query: String): List<Int> {
    if (query.length < 2) return emptyList()
    return rows.indices.filter { at -> rows[at].turns.any { turnMatches(it, query) } }
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
