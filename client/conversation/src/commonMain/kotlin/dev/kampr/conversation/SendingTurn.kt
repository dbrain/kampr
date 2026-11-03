package dev.kampr.conversation

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PendingSend
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn

// `turn.kind`s this client writes and the wire never carries, for a reply it has sent and the
// harness has not yet been seen to take. Not a record, and not queued either: nothing on the pane
// has said it has the words at all.
const val SENDING = "sending"
const val UNDELIVERED = "undelivered"
const val UNSEEN = "unseen"

fun sendingTurn(send: PendingSend): Turn = Turn(
    id = "sending:${send.id}",
    role = "user",
    blocks = listOf(Block.Md(send.text.replace("\u001b[200~", "").replace("\u001b[201~", " ").trim())),
    kind = when (send.state) {
        PendingSend.State.Sending -> SENDING
        PendingSend.State.Undelivered -> UNDELIVERED
        PendingSend.State.Unseen -> UNSEEN
    },
)

// Drawn as the user turn it is standing in for, so the record that replaces it lands where it is.
// The offer to try again is a box of its own under it, and only on a reply that needs one.
@Composable
fun SendingCard(
    send: PendingSend,
    agent: String?,
    now: Double,
    attachments: AttachmentStore,
    onResend: () -> Unit,
    onDismiss: () -> Unit,
) {
    val turn = sendingTurn(send)
    if (send.state == PendingSend.State.Sending) {
        TurnView(turn, "", emptyList(), {}, attachments = attachments, now = now, agent = agent)
        return
    }
    TurnView(turn, "", emptyList(), {}, attachments = attachments, now = now, agent = agent, edge = BlockEdge.Head)
    BlockFrame(speakerSkin(speakerOf(turn), agent), BlockEdge.Foot) {
        DisableSelection {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                QuietAction("Resend", onResend, label = "Resend this reply")
                QuietAction("Dismiss", onDismiss, label = "Dismiss this reply")
            }
        }
    }
}
