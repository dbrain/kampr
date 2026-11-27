package dev.kampr.conversation

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.wire.Abilities
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Task
import dev.kampr.shared.wire.Turn

// 2026-11-27T12:00:00Z, the one instant every seam test draws against.
const val SEAM_NOW = 1795780800000.0

val SEAM_HEADLESS = Abilities(
    input = listOf("slash", "mention", "queue_next", "send_now", "unqueue", "files", "images"),
    steer = listOf("interrupt"),
    asks = listOf("permission", "question", "plan"),
    streams = listOf("text", "task_output", "sub_live"),
    facts = listOf("account", "billing", "limits", "context"),
)

val SEAM_TASKS = listOf(
    Task("a1", "agent", title = "Explore — Map the manage op end-to-end path", since = "2026-11-27T11:42:00Z"),
    Task("s1", "shell", call = "cargo test --workspace -- --include-ignored 2>&1 | tee /tmp/test.log", parent = "a1", since = "2026-11-27T11:51:54Z"),
    Task("s2", "shell", call = "npm run dev", since = "2026-11-27T10:12:00Z", backgrounded = true),
    Task("m1", "monitor", title = "watch the release CI", state = "ended"),
)

val PLAN_ASK = ServerMsg.Pending(
    pane = PANE_ID,
    question = "Approve Claude's plan?",
    header = "Plan",
    kind = "plan",
    context = "## Steps\n\n" + (1..14).joinToString("\n") { "- step $it of the migration" },
    options = listOf(
        PendingOption("1", "Yes, and auto-accept edits"),
        PendingOption("2", "Yes, manually approve edits"),
        PendingOption("3", "No, keep planning"),
    ),
    source = "hook",
    id = "plan1",
)

val INDENT_ASK = ServerMsg.Pending(
    pane = PANE_ID,
    question = "Which indentation do you prefer?",
    header = "Indentation",
    options = listOf(
        PendingOption("1", "Tabs", "Indent with tab characters, one per level, as the formatter writes them."),
        PendingOption("2", "Two spaces", "Indent with two spaces per level, the way the rest of the tree reads."),
        PendingOption("3", "Four spaces", "Indent with four spaces per level, as the older crates still do."),
    ),
    source = "conversation",
    id = "q1",
)

fun seamPane(vararg frames: ServerMsg, abilities: Abilities = SEAM_HEADLESS, turns: Int = 2): PaneState = seamStore(*frames, abilities = abilities, turns = turns).pane(PANE_ID)

fun seamStore(vararg frames: ServerMsg, abilities: Abilities = SEAM_HEADLESS, turns: Int = 2): KamprStore {
    val store = KamprStore()
    store.accept(
        ServerMsg.Convo(
            PANE_ID,
            "t-1",
            false,
            (1..turns).map { n ->
                Turn("t-$n", if (n % 2 == 1) "user" else "assistant", null, listOf(Block.Md("Message $n, a few words long enough to be a line or two of a phone.")))
            },
        ),
    )
    store.accept(ServerMsg.ConvoAbilities(PANE_ID, abilities))
    frames.forEach { store.accept(it) }
    return store
}
