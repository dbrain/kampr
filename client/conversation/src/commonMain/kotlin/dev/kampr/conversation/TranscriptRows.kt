package dev.kampr.conversation

import androidx.compose.runtime.Composable
import dev.kampr.shared.net.wallClockMillis

// One row of a transcript, drawn the same way wherever the transcript came from. A conversation
// the pane's agent launched is a conversation in its own right and is read with the same eyes, so
// it is drawn by the same function rather than by a second one that will drift from this one.
@Composable
fun TranscriptRowView(
    rows: List<TranscriptRow>,
    at: Int,
    stamp: String?,
    query: String,
    expanded: List<String>,
    onToggle: (String) -> Unit,
    attachments: AttachmentStore,
    now: Double,
    agent: String?,
    clock: () -> Double = ::wallClockMillis,
) {
    val row = rows[at]
    val edge = blockEdge(rows.getOrNull(at - 1)?.block, row.block, rows.getOrNull(at + 1)?.block)
    when (row) {
        is TranscriptRow.Ask -> TurnView(
            row.turn, query, expanded, onToggle,
            attachments = attachments, now = now, agent = agent, edge = edge,
        )
        is TranscriptRow.Head -> ReplyHead(
            row.reply, agent, now,
            collapsed = row.key in expanded,
            onToggle = { onToggle(row.key) },
            edge = edge,
        )
        // A step is not a card of its own — it is content inside the one box its whole reply is
        // drawn as, and the box's own piece supplies the ground, the rail and the margins.
        is TranscriptRow.One -> TurnView(
            row.turn, query, expanded, onToggle,
            attachments = attachments, now = now,
            agent = agent, head = TurnHead.Stamp(stamp), edge = edge,
        )
        is TranscriptRow.Working -> BlockFrame(speakerSkin(Speaker.Agent, agent), edge) {
            WorkingStrip(row.reply, clock = clock)
        }
        is TranscriptRow.Run -> BlockFrame(speakerSkin(Speaker.Agent, agent), edge) {
            StepStamp(stamp)
            ToolRunCard(row, row.key in expanded, { onToggle(row.key) }) {
                for (turn in row.turns) {
                    TurnView(
                        turn, query, expanded, onToggle,
                        attachments = attachments, now = now,
                        agent = agent, framed = false, head = TurnHead.None,
                    )
                }
            }
        }
    }
}
