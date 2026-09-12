package dev.kampr.conversation

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
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

// One result, whichever half of the search found it: the node over the whole transcript, or this
// client over the turns it holds. Both aim by **turn id** — the node's hits name turns this client
// may not hold yet, and a row index would mean nothing until it did.
//
// `fromEnd` is how many turns back the hit is, which only the node can answer; a local result
// leaves it absent rather than counting the window it happens to hold as the transcript.
data class Result(
    val turn: String,
    val role: String,
    val at: String?,
    val text: String,
    val hits: Int,
    val fromEnd: Int?,
)

// Oldest first, both ways round: it is the order the transcript is in, so stepping down the list is
// stepping down the screen. The node answers newest first, which is the end a reader stands at.
fun resultsOf(found: ServerMsg.ConvoFound?, rows: List<TranscriptRow>, query: String): List<Result> {
    if (query.length < 2) return emptyList()
    val here = localResults(rows, query)
    if (found == null) return here
    val named = found.matches.mapTo(HashSet()) { it.turn }
    // **The node's answer, and then the turns the node cannot have seen.** The message being
    // written now is read off the *screen* and the queue is read off the facets — neither is in
    // the transcript that was searched — so a hit in the answer appearing under the reader's eyes
    // was highlighted on screen and counted by nobody. They go on the end because that is where
    // they are: the newest things on the transcript, and so the first place a search lands.
    return found.matches.reversed().map {
        Result(it.turn, it.role, it.at, it.text, it.hits.coerceAtLeast(1), it.fromEnd)
    } + here.filterNot { it.turn in named }
}

private fun localResults(rows: List<TranscriptRow>, query: String): List<Result> =
    rows.mapNotNull { row ->
        val turn = row.turns.firstOrNull { turnMatches(it, query) } ?: return@mapNotNull null
        val text = turnText(turn)
        Result(
            turn = turn.id,
            role = turn.role,
            at = turn.at,
            text = matchLine(text, query),
            hits = matchRanges(text, query).size,
            fromEnd = null,
        )
    }

// Characters of the matched line worth showing, and how much of it is kept in front of the match.
// The node clips its own excerpts to the same shape and for the same reason: a tool's output is one
// block and can be thousands of columns, and a result list is only a list while its rows are lines.
private const val EXCERPT = 200
private const val LEAD = 60

fun matchLine(text: String, query: String): String {
    val at = text.indexOf(query, 0, ignoreCase = true).takeIf { it >= 0 } ?: return text.take(EXCERPT).trim()
    val start = text.lastIndexOf('\n', at).let { if (it < 0) 0 else it + 1 }
    val end = text.indexOf('\n', at).let { if (it < 0) text.length else it }
    val line = text.substring(start, end).trim()
    if (line.length <= EXCERPT) return line
    // Counted from the match rather than from the line, so a hit four thousand columns in is still
    // in the middle of what comes back.
    val from = (at - LEAD).coerceAtLeast(start)
    val cut = text.substring(from, (from + EXCERPT - 2).coerceAtMost(end))
    return buildString {
        if (from > start) append('…')
        append(cut.trimEnd())
        if (from + cut.length < end) append('…')
    }
}
