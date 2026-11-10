package dev.kampr.shared.model

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.mutableStateListOf
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Queued
import dev.kampr.shared.wire.Turn
import kotlin.random.Random

// How long a sent reply may go without its record before it is flagged. Generous on purpose: a
// harness's first record of a session is written only after it has finished starting and taken the
// prompt up, and a reply to a pane that is still booting is exactly the one this card is for.
const val UNSEEN_AFTER_MS = 45_000L

@Immutable
class PendingSend(
    val id: String,
    val text: String,
    val state: State,
    val at: Double,
    internal val key: String,
    internal val known: Set<String>,
    internal val queuedBefore: Int,
    // The replies already on the pane when the harness said it took this one: any other is the
    // answer to it, which retires a send whose own record never matches (a slash command's).
    internal val replies: Set<String> = emptySet(),
) {
    enum class State { Sending, Undelivered, Unseen, Delivered }

    internal fun with(state: State, replies: Set<String> = this.replies) = PendingSend(id, text, state, at, key, known, queuedBefore, replies)
}

// Replies this client has sent and the harness has not yet been seen to take, held on the pane
// beside the draft so a glance at the terminal does not lose them.
//
// **A record retires one send, once.** Two identical replies are two records, so each record is
// spent on the oldest send it matches and never again, and a record already in the transcript when
// the send went is never one of them.
class PendingSends {
    private val sends = mutableStateListOf<PendingSend>()

    // A send retired by a queued entry is still owed its record, and that record must not be taken
    // for a later identical send.
    private val owed = mutableListOf<PendingSend>()
    private val spent = mutableSetOf<String>()

    val all: List<PendingSend> get() = sends

    fun sent(text: String, turns: List<Turn>, queued: List<Queued>, delivered: Boolean, at: Double): PendingSend {
        val send = fresh(Random.nextLong().toULong().toString(36), text, turns, queued, delivered, at)
        sends += send
        return send
    }

    fun resent(id: String, turns: List<Turn>, queued: List<Queued>, delivered: Boolean, at: Double) {
        val index = sends.indexOfFirst { it.id == id }
        if (index >= 0) sends[index] = fresh(id, sends[index].text, turns, queued, delivered, at)
    }

    fun unseen(id: String) {
        val at = sends.indexOfFirst { it.id == id && it.state == PendingSend.State.Sending }
        if (at >= 0) sends[at] = sends[at].with(PendingSend.State.Unseen)
    }

    // The harness saying it took a prompt (`convo.heard`). Its record still retires the send, so a
    // delivered one stays until then — drawn as the operator's turn rather than as a send.
    fun acknowledged(text: String, turns: List<Turn>) {
        val key = sentKey(text)
        val at = sends.indexOfFirst { it.key == key && it.state != PendingSend.State.Delivered }
        if (at >= 0) sends[at] = sends[at].with(PendingSend.State.Delivered, replies = repliesIn(turns))
    }

    fun expired(id: String) {
        sends.removeAll { it.id == id && it.state == PendingSend.State.Delivered }
    }

    fun dismiss(id: String) {
        sends.removeAll { it.id == id }
    }

    fun heard(turns: List<Turn>, queued: List<Queued>) {
        if (sends.isEmpty() && owed.isEmpty()) {
            spent.clear()
            return
        }
        for (turn in turns) {
            if (turn.role != "user" || turn.kind != null || turn.id == LIVE_TURN_ID || turn.id in spent) continue
            val key = recordedKey(turn)
            val claim = owed.firstOrNull { it.key == key && turn.id !in it.known }
                ?: sends.firstOrNull { it.key == key && turn.id !in it.known }
                ?: continue
            spent += turn.id
            owed.remove(claim)
            sends.remove(claim)
        }
        sends.removeAll { it.state == PendingSend.State.Delivered && answered(it, turns) }
        for (key in sends.map { it.key }.distinct()) {
            val waiting = queued.count { sentKey(it.text) == key } - owed.count { it.key == key }
            var taken = 0
            for (send in sends.filter { it.key == key }) {
                if (waiting - send.queuedBefore - taken <= 0) break
                taken++
                sends.remove(send)
                owed += send
            }
        }
    }

    private fun fresh(id: String, text: String, turns: List<Turn>, queued: List<Queued>, delivered: Boolean, at: Double): PendingSend {
        val key = sentKey(text)
        return PendingSend(
            id = id,
            text = text,
            state = if (delivered) PendingSend.State.Sending else PendingSend.State.Undelivered,
            at = at,
            key = key,
            known = turns.filter { it.role == "user" }.mapTo(HashSet()) { it.id },
            queuedBefore = queued.count { sentKey(it.text) == key },
        )
    }
}

// After the last reply it knew of, so a page of older turns paged in above is not the answer.
private fun answered(send: PendingSend, turns: List<Turn>): Boolean {
    val since = turns.indexOfLast { it.id in send.replies }
    return turns.drop(since + 1).any { it.role != "user" && it.id != LIVE_TURN_ID && it.id !in send.replies }
}

private fun repliesIn(turns: List<Turn>): Set<String> = turns.filter { it.role != "user" && it.id != LIVE_TURN_ID }.mapTo(HashSet()) { it.id }

private const val BRACKETED_CLOSE = "\u001b[201~"
private val IMAGE_MARKER = Regex("""^\[Image #\d+]$""")
private val MUFFIT_LABEL = Regex("""\[image \d+: [^\]]*]""")

// What a reply and its record have in common. The node lifts a picture's path out of a muffit
// reply and leaves `[image n: file]` where it stood, Claude writes an image it took from a path as
// `[Image #n]`, and a harness keeps or drops a trailing newline as it likes — so all of that goes,
// and what is left is the words, with their spacing collapsed.
fun sentKey(text: String): String = text
    .replace(MUFFIT_LABEL, " ")
    .replace(BRACKETED_PASTE, " ")
    .replace(BRACKETED_CLOSE, " ")
    .split(' ', '\t', '\n', '\r')
    .filter { it.isNotEmpty() && "/pastes/" !in it && !IMAGE_MARKER.matches(it) }
    .joinToString(" ")

private fun recordedKey(turn: Turn): String = sentKey(
    turn.blocks.filterIsInstance<Block.Md>().filter { it.att == null }.joinToString(" ") { it.text },
)
