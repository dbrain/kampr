package dev.kampr.shared.model

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.ResumeRecord
import dev.kampr.shared.wire.ServerMsg

private const val ASK = "resume-ask:"
private const val RUN = "resume-run:"
private const val DROP = "resume-drop:"

// docs/25's Resume after reboot. Held by the app, not the herd list, so an answer that lands while
// the list is not drawn is still there when it is. An ack names no node, so each ask carries its
// machine in the `rid`.
//
// Asked once per machine per socket rather than per herd frame, which arrive with every status. A
// machine that drops out of the herd and comes back may have rebooted, so it is asked again.
@Stable
class ResumeOffers(
    private val send: (ClientMsg) -> Unit,
    private val opened: (String?) -> Unit,
    private val refused: (ServerMsg.Managed) -> Unit,
) {
    private var offered by mutableStateOf<Map<String, List<ResumeRecord>>>(emptyMap())

    private val asked = HashSet<String>()

    fun on(node: String): List<ResumeRecord> = offered[node].orEmpty()

    fun herd(nodes: List<NodeInfo>) {
        val keeping = nodes.filter { it.resume == true && it.isReachable }.map { it.id }.toSet()
        asked.retainAll(keeping)
        offered = offered.filterKeys { it in keeping }
        for (node in keeping) {
            if (asked.add(node)) send(ClientMsg.Manage(ManageOp.NativeResumable(node), ASK + node))
        }
    }

    fun disconnected() {
        asked.clear()
        offered = emptyMap()
    }

    fun resume(node: String, record: ResumeRecord) {
        spend(node, record)
        send(ClientMsg.Manage(ManageOp.NativeResume(node, record.id), RUN + record.id))
    }

    fun dismiss(node: String, record: ResumeRecord) {
        spend(node, record)
        send(ClientMsg.Manage(ManageOp.NativeResumeDismiss(node, record.id), DROP + record.id))
    }

    private fun spend(node: String, record: ResumeRecord) {
        offered = offered + (node to on(node).filterNot { it.id == record.id })
    }

    fun acked(ack: ServerMsg.Managed) {
        val rid = ack.rid ?: return
        when {
            rid.startsWith(ASK) -> {
                val node = rid.removePrefix(ASK)
                if (node in asked) offered = offered + (node to ack.resumeRecords.orEmpty().takeIf { ack.ok }.orEmpty())
            }

            rid.startsWith(RUN) -> if (ack.ok) opened(ack.id) else refused(ack)

            rid.startsWith(DROP) -> if (!ack.ok) refused(ack)
        }
    }
}
