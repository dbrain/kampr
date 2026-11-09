package dev.kampr.shared.model

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.wire.DockerAction
import dev.kampr.shared.wire.DockerDetail
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.dockerDetailOf
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

const val DOCKER_DETAIL_PERIOD_MS = 3_000L

// The detail sheet's live half (docs/19): `docker.detail` on open and every three seconds while it
// stays open. Held by the app rather than the sheet, so the poll stops with the sheet however it
// closes and never outlives it in a composition nobody is drawing.
//
// The acks are read off the unconflated stream: a poll answering in the same frame as a shell or a
// refusal would otherwise replace it in the store's single slot before the sheet saw it.
class ContainerWatch(
    private val scope: CoroutineScope,
    private val send: (ManageOp) -> Unit,
    private val shellOpened: (String?) -> Unit = {},
) {
    var watching: Pair<String, String>? by mutableStateOf(null)
        private set

    var detail: DockerDetail? by mutableStateOf(null)
        private set

    var refusal: String? by mutableStateOf(null)
        private set

    private var poll: Job? = null

    fun act(op: ManageOp) {
        refusal = null
        send(op)
    }

    fun watch(node: String, id: String) {
        if (watching == node to id) return
        stop()
        watching = node to id
        poll = scope.launch {
            while (true) {
                send(ManageOp.Docker(DockerAction.Detail, node, id))
                delay(DOCKER_DETAIL_PERIOD_MS)
            }
        }
    }

    fun refresh() {
        val (node, id) = watching ?: return
        send(ManageOp.Docker(DockerAction.Detail, node, id))
    }

    fun stop() {
        poll?.cancel()
        poll = null
        watching = null
        detail = null
        refusal = null
    }

    fun acked(ack: ServerMsg.Managed) {
        val (_, id) = watching ?: return
        when {
            !ack.op.startsWith("docker.") -> Unit

            !ack.ok -> refusal = ack.message ?: ack.code ?: "${ack.op} was refused."

            ack.op == "docker.detail" -> dockerDetailOf(ack.detail)
                ?.takeIf { it.entry.id.isEmpty() || it.entry.id == id }
                ?.let { detail = it }

            ack.op == "docker.shell" -> shellOpened(ack.id)

            else -> refresh()
        }
    }
}
