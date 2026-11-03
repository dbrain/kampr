package dev.kampr.shared.model

import dev.kampr.shared.wire.EditKeys

internal val IMAGE_TOKEN = Regex("""\[Image #\d+]""")

internal const val BRACKETED_PASTE = "\u001b[200~"

// What one key moves over between `from` and `to`: a character, or a whole token on a harness
// that draws a picture as one (#620).
internal fun steps(keys: EditKeys, text: String, from: Int, to: Int): Int {
    if (!keys.images) return to - from
    return IMAGE_TOKEN.findAll(text)
        .filter { it.range.first >= from && it.range.last < to }
        .fold(to - from) { left, token -> left - (token.value.length - 1) }
}

// The edit the reply box makes instead, when one cuts into a token: the harness takes a token
// whole and puts nothing inside one, so the box does the same and stays the pane's line. A label
// standing for a held file is whole in the same way. The text
// and the caret, or null for an edit that leaves every token whole.
fun keepTokensWhole(before: String, after: String, labels: List<String> = emptyList()): Pair<String, Int>? {
    val shared = before.commonPrefixWith(after).length
    val tail = before.substring(shared).commonSuffixWith(after.substring(shared)).length
    var start = shared
    var end = before.length - tail
    val inserted = after.substring(shared, after.length - tail)
    val tokens = IMAGE_TOKEN.findAll(before).map { it.range.first until it.range.last + 1 }.toList() +
        labels.flatMap { label -> Regex.escape(label).toRegex().findAll(before).map { it.range.first until it.range.last + 1 } }
    if (start == end) {
        val inside = tokens.firstOrNull { start > it.first && start < it.last + 1 } ?: return null
        start = inside.last + 1
        end = start
    } else {
        val cut = tokens.filter { it.first < end && it.last + 1 > start && !(start <= it.first && it.last + 1 <= end) }
        if (cut.isEmpty()) return null
        start = minOf(start, cut.minOf { it.first })
        end = maxOf(end, cut.maxOf { it.last + 1 })
    }
    return before.substring(0, start) + inserted + before.substring(end) to start + inserted.length
}
