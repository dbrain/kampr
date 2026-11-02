package dev.kampr.shared.model

import androidx.compose.runtime.mutableStateListOf
import dev.kampr.shared.wire.ServerMsg
import kotlin.random.Random

class PendingFile(val id: String, val name: String, val bytes: ByteArray, val state: State) {
    sealed interface State {
        data object Sending : State

        data class Ready(val path: String, val text: String) : State

        data class Refused(val reason: String) : State
    }

    fun with(state: State) = PendingFile(id, name, bytes, state)
}

// Files the conversation's composer holds for its next reply. On the pane rather than in the
// composer, beside the draft, because the composer leaves the composition whenever the terminal is
// looked at and a file the node has already written must not be forgotten by a glance.
class PendingFiles {
    private val files = mutableStateListOf<PendingFile>()

    val all: List<PendingFile> get() = files

    // Random rather than counted: through a hub every client watching the pane hears the answer,
    // and the id is how this one tells its own from theirs.
    fun add(name: String, bytes: ByteArray, state: PendingFile.State = PendingFile.State.Sending): PendingFile {
        val file = PendingFile(Random.nextLong().toULong().toString(36), name, bytes, state)
        files += file
        return file
    }

    fun answered(msg: ServerMsg.Pasted) {
        val state = when {
            msg.refused != null -> PendingFile.State.Refused(msg.refused)
            msg.path != null -> PendingFile.State.Ready(msg.path, msg.text ?: "${msg.path} ")
            else -> return
        }
        move(msg.id ?: return, state)
    }

    // A pane error while a file is on its way is the only word there will be about it: a hub that
    // could not reach the peer, or a device that may not type, answers with no id.
    fun refuseSending(reason: String) {
        files.filter { it.state == PendingFile.State.Sending }.forEach { move(it.id, PendingFile.State.Refused(reason)) }
    }

    fun refuse(id: String, reason: String) = move(id, PendingFile.State.Refused(reason))

    fun remove(id: String) {
        files.removeAll { it.id == id }
    }

    // What the reply says for the files it carries, which then leave. A refusal stays: it is the
    // only report that a file never arrived, and a send that had nothing to do with it is no reason
    // to sweep it away unread.
    fun take(): String {
        val ready = files.filter { it.state is PendingFile.State.Ready }
        files.removeAll(ready)
        return ready.joinToString("") { (it.state as PendingFile.State.Ready).text }
    }

    private fun move(id: String, state: PendingFile.State) {
        val at = files.indexOfFirst { it.id == id }
        if (at >= 0) files[at] = files[at].with(state)
    }
}
