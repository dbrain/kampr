package dev.kampr.shared.model

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.ServerMsg
import kotlin.random.Random

private val OFFERED_STATES = setOf("absent", "stale")

// docs/12 W11: whether Kampr's hooks are in this Claude's settings, and the one install or decline
// this client has asked for. Held on the pane so the answer reaches the conversation, which cannot
// see the app's ack stream.
@Stable
class ClaudeHooksOffer {
    var state by mutableStateOf<ServerMsg.ClaudeHooks?>(null)
        private set

    private var rid by mutableStateOf<String?>(null)

    var refusal by mutableStateOf<String?>(null)
        private set

    // An install the node took is not offered again before the node's next `claude.hooks` says
    // what it now holds, whichever of the two arrives first.
    private var settled by mutableStateOf(false)

    val asking: Boolean get() = rid != null

    fun offered(readOnly: Boolean): ServerMsg.ClaudeHooks? = state?.takeIf {
        !readOnly && !settled && it.ask && it.state in OFFERED_STATES
    }

    fun ask(op: ManageOp, send: (ClientMsg) -> Unit) {
        val token = "hooks-" + Random.nextLong().toULong().toString(36)
        rid = token
        refusal = null
        send(ClientMsg.Manage(op, token))
    }

    internal fun heard(msg: ServerMsg.ClaudeHooks) {
        state = msg
        settled = false
    }

    internal fun acked(ack: ServerMsg.Managed): Boolean {
        if (ack.rid == null || ack.rid != rid) return false
        rid = null
        if (ack.ok) {
            settled = true
        } else {
            refusal = ack.message ?: ack.code ?: "${ack.op} was refused."
        }
        return true
    }

    internal fun dropped() {
        rid = null
    }
}
