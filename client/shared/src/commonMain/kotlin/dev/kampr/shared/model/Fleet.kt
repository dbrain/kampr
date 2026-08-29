package dev.kampr.shared.model

import androidx.compose.runtime.Immutable
import kotlin.random.Random
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo

// One fan-out: the same command, on however many hosts it was sent to.
//
// Kept in step with `kampr_client::herd::Cohort` and its Rust tests — the two clients must not
// disagree about which host needs somebody, and the ordering here is the whole reason the board is
// readable at a glance.
@Immutable
data class Cohort(
    val id: String,
    val command: String,
    val startedUnix: Long,
    // Needs-you first, then still going, then quiet, then failures, then successes.
    val panes: List<PaneInfo>,
) {
    private val fleets get() = panes.mapNotNull { it.fleet }

    val waiting: Int get() = fleets.count { it.state == "waiting" }
    val running: Int get() = fleets.count { it.state == "running" }
    val quiet: Int get() = fleets.count { it.state == "quiet" }
    val succeeded: Int get() = fleets.count { it.succeeded }
    val failed: Int get() = fleets.count { it.failed }
    val finished: Boolean get() = fleets.isNotEmpty() && fleets.all { it.isFinished }
}

// The board's order. Failures sort **above** successes among the finished, because the finished
// half of the board is read to find out what went wrong.
private fun boardRank(pane: PaneInfo): Int {
    val fleet = pane.fleet ?: return 9
    return when {
        fleet.state == "waiting" -> 0
        fleet.state == "running" -> 1
        fleet.state == "quiet" -> 2
        fleet.succeeded -> 4
        fleet.isFinished -> 3
        else -> 5
    }
}

private val boardOrder = compareBy<PaneInfo>({ boardRank(it) }, { it.id })

// Every fleet run, gathered into the fan-outs that produced them, newest first.
fun Herd.cohorts(): List<Cohort> =
    panes.mapNotNull { pane -> pane.fleet?.let { pane to it } }
        .groupBy { (_, fleet) -> fleet.cohort }
        .map { (id, pairs) ->
            val ordered = pairs.map { it.first }.sortedWith(boardOrder)
            Cohort(
                id = id,
                command = pairs.first().second.command,
                startedUnix = pairs.minOf { it.second.startedUnix },
                panes = ordered,
            )
        }
        .sortedWith(compareByDescending<Cohort> { it.startedUnix }.thenBy { it.id })

// Which other hosts in this run are asking exactly the same thing.
@Immutable
data class Matching(
    val target: PaneInfo,
    val others: List<PaneInfo>,
    // Waiting panes in the cohort asking something else, named so the operator can see what is
    // *not* being answered. The silent third of a fleet is what bites you.
    val differing: List<PaneInfo>,
) {
    val reach: Int get() = 1 + others.size
}

enum class AnswerRefusal { NotAFleetRun, NotWaiting, Secret }

// The other hosts one answer would reach, or why it would reach none.
//
// **Byte-identical, not merely similar.** The prompt, the shape, the options and their order all
// have to match, because "these two look alike" is exactly the reasoning that sends `y` to the host
// that was asking something else.
fun Herd.matching(paneId: String): Result<Matching> {
    val target = panes.firstOrNull { it.id == paneId }
        ?: return Result.failure(FleetRefused(AnswerRefusal.NotAFleetRun))
    val fleet = target.fleet ?: return Result.failure(FleetRefused(AnswerRefusal.NotAFleetRun))
    val question = fleet.question ?: return Result.failure(FleetRefused(AnswerRefusal.NotWaiting))
    if (!fleet.isWaiting) return Result.failure(FleetRefused(AnswerRefusal.NotWaiting))
    // A password sent to five hosts because five prompts said "Password:" is a password given to
    // whichever of them was asking for something else. The text is no evidence at all here — every
    // one of them says the same word.
    if (question.isSecret) return Result.failure(FleetRefused(AnswerRefusal.Secret))

    val others = mutableListOf<PaneInfo>()
    val differing = mutableListOf<PaneInfo>()
    for (pane in panes) {
        if (pane.id == target.id) continue
        val other = pane.fleet ?: continue
        if (other.cohort != fleet.cohort) continue
        val theirs = other.question ?: continue
        if (theirs == question && other.command == fleet.command) others += pane else differing += pane
    }
    return Result.success(Matching(target, others, differing))
}

class FleetRefused(val refusal: AnswerRefusal) : Exception(
    when (refusal) {
        AnswerRefusal.NotAFleetRun -> "that pane is not a fleet run"
        AnswerRefusal.NotWaiting -> "that host is not waiting for anything"
        AnswerRefusal.Secret -> "a password is answered one host at a time"
    },
)

// Which panes one answer should be typed into. The answer itself travels as the ordinary pane
// input every other reply uses — there is no `fleet.answer`, and a second way to type into a
// terminal would be a second thing to get wrong.
fun Matching.recipients(): List<String> = listOf(target.id) + others.map { it.id }

// One `fleet.run` per reachable node, all carrying the same cohort.
//
// **Reachable, not `online`.** `online` is the node's herdr health and a fleet run needs no herdr —
// a machine whose herdr is down runs commands perfectly well.
fun fleetTargets(nodes: List<NodeInfo>): List<NodeInfo> = nodes.filter { it.isReachable }

// Splits on whitespace, honouring quotes. **Not a shell, and deliberately not one**: nothing runs
// `sh -c` for this, so a `;` or a `&&` is an argument rather than something that runs on every
// machine in the herd. Kept in step with `kampr_client::fleet::split`.
fun splitCommand(command: String): List<String>? {
    val argv = mutableListOf<String>()
    val current = StringBuilder()
    var started = false
    var quote: Char? = null
    for (c in command) {
        when {
            quote != null && c == quote -> quote = null
            quote != null -> current.append(c)
            c == '\'' || c == '"' -> {
                quote = c
                started = true
            }
            c.isWhitespace() -> if (started) {
                argv += current.toString()
                current.clear()
                started = false
            }
            else -> {
                current.append(c)
                started = true
            }
        }
    }
    if (quote != null) return null
    if (started) argv += current.toString()
    return argv
}

// A name for one fan-out. Only two things are asked of it: that two runs started seconds apart do
// not collide, and that it sorts by time — the board shows the newest run first, and reads that
// from the panes' own `started_unix` rather than from this, so a clock that disagrees costs an
// ordering and never a wrong grouping.
fun newCohortId(millis: Long = 0L): String {
    val stamp = (if (millis > 0) millis else Random.nextLong(1L shl 44)).toString(36).padStart(9, '0')
    val tail = (0 until 8).map { ALPHABET[Random.nextInt(ALPHABET.length)] }.joinToString("")
    return stamp + tail
}

private const val ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
