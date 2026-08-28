package dev.kampr.conversation

import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Facets
import dev.kampr.shared.wire.Turn

// A `turn.kind` this client writes and the wire never carries: a prompt the harness has queued and
// not started on.
//
// **It is not a record and must not read as one.** The operator typing while an agent is mid-turn
// is asking one question — has it got my message — and a card indistinguishable from a turn the
// harness has taken up answers it wrongly. So it takes the shape the live preview took from the
// other direction: a turn under a reserved id, standing where the record will be, marked as not
// yet one. `kind` is the mechanism because it is already the one the summary fold uses (#259) for
// exactly this — a turn that is not what its role says.
const val QUEUED = "queued"

fun isQueued(turn: Turn): Boolean = turn.kind == QUEUED

// The queue as turns, in the order the harness will take them.
//
// Keyed by position, because position is the only identity a queued prompt has: the harness
// records no id for one, and it works the queue by position rather than by text (#320). A prompt
// leaving the head therefore renumbers the ones behind it, which costs the re-composition of a
// card or two and is the only thing on offer that is unique for two identical prompts.
fun queuedTurns(facets: Facets): List<Turn> = facets.queued.mapIndexed { at, queued ->
    Turn("queued:$at", "user", queued.at, listOf(Block.Md(queued.text)), QUEUED)
}
