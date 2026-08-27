package dev.kampr.conversation

import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn

// Everything the agent did between one thing the operator said and the next. It is the unit a
// reader thinks in — "that answer", not "the fourth turn of that answer" — and it is the unit they
// want to put away: what is worth hiding is a whole reply and the thirty tool calls inside it,
// never one of the thirty on its own.
//
// A transcript paged backwards can open in the middle of one, so a reply with nothing in front of
// it is a reply, not a broken exchange.
class Reply(val turns: List<Turn>) {
    val key: String = "reply:${turns.first().id}"
    val at: String? = turns.firstNotNullOfOrNull { it.at }
    val until: String? = turns.asReversed().firstNotNullOfOrNull { it.at }
    val steps: Int = turns.size
    val tools: Int = turns.sumOf { turn -> turn.blocks.count { it is Block.Tool } }
    val live: Boolean = turns.any { it.id == LIVE_TURN_ID }
}

// The line a put-away reply shows of itself. Its first *prose*, not its first turn: a reply that
// opens on three tool calls would otherwise describe itself as "Bash", which is what it did rather
// than what it said.
fun replyGist(reply: Reply): String =
    reply.turns.firstOrNull { turn -> turn.blocks.any { it is Block.Md && it.att == null } }
        ?.let(::turnGist)
        ?: reply.turns.firstOrNull()?.let(::turnGist).orEmpty()

// Every adapter writes one tool call per record and carries that through as one turn, so a run of
// calls is a run of *turns* — grouping inside a turn would never fire on a real transcript. A turn
// that does hold several at once, from a harness that batches them, joins the same run and is
// counted by its calls rather than by itself.
sealed interface TranscriptRow {
    val key: String
    val turns: List<Turn>

    // The block this row sits inside: the reply it is one step of, or an ask, which is its own.
    // What the pinned header names, and what collapsing from that header puts away.
    val block: String

    data class Ask(val turn: Turn) : TranscriptRow {
        override val key: String = turn.id
        override val turns: List<Turn> = listOf(turn)
        override val block: String = turn.id
    }

    // `turns` is empty on purpose: a search counts rows, and a reply whose header and whose steps
    // both answered would be counted twice and stepped through twice for one match. The head is
    // reached by opening the reply, which a match does by itself.
    data class Head(val reply: Reply) : TranscriptRow {
        override val key: String = reply.key
        override val turns: List<Turn> = emptyList()
        override val block: String = reply.key
    }

    data class One(val turn: Turn, override val block: String) : TranscriptRow {
        override val key: String = turn.id
        override val turns: List<Turn> = listOf(turn)
    }

    data class Run(override val turns: List<Turn>, override val block: String) : TranscriptRow {
        override val key: String = "run:${turns.first().id}"
        val tools: List<Block.Tool> = turns.flatMap { turn -> turn.blocks.filterIsInstance<Block.Tool>() }
    }

    // The progress line, which is a piece of the reply it belongs to rather than something under
    // it: an agent still working is not finished, and a box drawn shut below a running answer says
    // it is.
    data class Working(val reply: Reply) : TranscriptRow {
        override val key: String = "working"
        override val turns: List<Turn> = emptyList()
        override val block: String = reply.key
    }
}

// A turn that says anything of its own ends a run of calls, and that includes a turn that speaks as
// well as calling something: a collapsed run would take the sentence down with the call. A code
// fence or a patch that is a call's *own* output is already part of that call and ends nothing.
private fun callsIn(turn: Turn): Int {
    val pieces = groupBlocks(turn.blocks)
    return if (pieces.isNotEmpty() && pieces.all { it is Piece.Call }) pieces.size else 0
}

fun transcriptRows(turns: List<Turn>, query: String, collapsed: List<String> = emptyList()): List<TranscriptRow> {
    val out = mutableListOf<TranscriptRow>()
    var at = 0
    while (at < turns.size) {
        if (turns[at].role == "user") {
            out += TranscriptRow.Ask(turns[at])
            at++
            continue
        }
        val from = at
        while (at < turns.size && turns[at].role != "user") at++
        val reply = Reply(turns.subList(from, at).toList())
        out += TranscriptRow.Head(reply)
        // A put-away reply holding what the search is looking for opens itself, for the reason a
        // run of tool calls does: a hit the counter promises and the screen hides is worse than a
        // screen that is too long.
        val holds = reply.turns.any { turnMatches(it, query) }
        if (reply.key !in collapsed || holds) out += replyRows(reply, query)
    }
    return out
}

// The clock each piece shows, and nothing shows one that repeats the line above it. A harness
// writing four records inside the same minute stamped all four the same, which is four rows of
// chrome saying one thing — and the head has already said when the reply began, so the first step
// landing in that minute is the fifth.
//
// Blank rather than absent: a step whose stamp is suppressed still had one, and the *next* step to
// differ is measured against the last one shown rather than against its own neighbour.
fun stepStamps(rows: List<TranscriptRow>, nowMillis: Double): List<String?> {
    val out = ArrayList<String?>(rows.size)
    var block: String? = null
    var last: String? = null
    for (row in rows) {
        if (row.block != block) {
            block = row.block
            last = null
        }
        val stamp = when (row) {
            is TranscriptRow.Head -> turnStamp(row.reply.at, nowMillis).also { last = it; out += null }
            is TranscriptRow.One -> turnStamp(row.turn.at, nowMillis)
            is TranscriptRow.Run -> turnStamp(row.turns.first().at, nowMillis)
            else -> null.also { out += null }
        }
        if (row is TranscriptRow.One || row is TranscriptRow.Run) {
            out += stamp.takeIf { it != last }
            if (stamp != null) last = stamp
        }
    }
    return out
}

private fun replyRows(reply: Reply, query: String): List<TranscriptRow> {
    val out = mutableListOf<TranscriptRow>()
    val turns = reply.turns
    var at = 0
    while (at < turns.size) {
        var end = at
        var calls = 0
        while (end < turns.size) {
            val more = callsIn(turns[end])
            if (more == 0) break
            calls += more
            end++
        }
        val run = turns.subList(at, end)
        val hides = run.any { turnMatches(it, query) }
        if (calls >= TOOL_RUN_MIN && !hides) {
            out += TranscriptRow.Run(run.toList(), reply.key)
            at = end
        } else {
            out += TranscriptRow.One(turns[at], reply.key)
            at++
        }
    }
    return out
}
