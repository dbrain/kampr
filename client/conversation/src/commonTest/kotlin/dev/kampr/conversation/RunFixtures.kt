package dev.kampr.conversation

import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn

fun bashTurn(id: String, command: String, state: String, lines: Int?): Turn = Turn(
    id = id,
    role = "assistant",
    at = null,
    blocks = listOf(Block.Tool("Bash", command, lines, state), Block.Code("bash", command)),
)

fun proseTurn(id: String, text: String, role: String = "assistant"): Turn =
    Turn(id, role, null, listOf(Block.Md(text)))

// The report's own shape: a wall of `Bash` cards with one sentence of prose in the middle of it,
// which is what makes it two runs rather than one. One run hides a failure and the other hides a
// call still in flight, because those are the two things a collapsed row must say out loud.
val TOOL_RUN_TURNS: List<Turn> = listOf(
    proseTurn("r-1", "check the tests and the lint", role = "user"),
    bashTurn("r-2", "cargo fmt --all -- --check", "done", 1),
    bashTurn("r-3", "cargo clippy --workspace --all-targets", "error", 12),
    bashTurn("r-4", "cargo test -p kampr-term", "done", 40),
    proseTurn("r-5", "Clippy is unhappy about the width inference. I will run the rest before I touch it."),
    bashTurn("r-6", "cargo test -p kampr-core", "done", 61),
    Turn("r-7", "assistant", null, listOf(Block.Tool("Read", "crates/kampr-core/src/width.rs", 74, "done"))),
    bashTurn("r-8", "cargo test -p kampr-node", "running", null),
)

// What the view does with a query, in the order it does it: rows first, and the hits are indices
// into those rows rather than into the turns behind them.
fun hitRows(turns: List<Turn>, query: String): List<Int> = searchHits(transcriptRows(turns, query), query)
