package dev.kampr.shared.model

import androidx.compose.runtime.mutableStateListOf
import dev.kampr.shared.wire.ServerMsg
import kotlin.random.Random

// How long a file put into the pane waits for the node to say where it landed. A node, or a hub
// relaying to one, from before files were placed types the file and never answers; a refusal
// still comes, as a pane error, so a file unanswered this long went in.
const val PLACED_ANSWER_WAIT_MS = 15_000L

// `inline` is a file the node types into the pane at its caret, where the harness draws it: once
// the node says where it landed, the line carries it and the chip goes. Any other file is held as
// `label` in the reply box, at the caret it was attached at, until the reply goes.
class PendingFile(
    val id: String,
    val name: String,
    val bytes: ByteArray,
    val state: State,
    val inline: Boolean = false,
    val label: String? = null,
    val since: Double? = null,
) {
    sealed interface State {
        data object Sending : State

        data class Ready(val path: String, val text: String) : State

        data class Refused(val reason: String) : State
    }

    fun with(state: State, since: Double? = this.since) = PendingFile(id, name, bytes, state, inline, label, since)
}

// A change to the reply box that comes from outside it, applied by the composer the next time it
// is composed.
sealed interface BoxEdit {
    val label: String

    data class Insert(override val label: String) : BoxEdit

    data class Remove(override val label: String) : BoxEdit
}

// Files the conversation's composer holds for its next reply. On the pane rather than in the
// composer, beside the draft, because the composer leaves the composition whenever the terminal is
// looked at and a file the node has already written must not be forgotten by a glance.
class PendingFiles {
    private val files = mutableStateListOf<PendingFile>()
    private var labelled = 0

    val all: List<PendingFile> get() = files

    val edits = mutableStateListOf<BoxEdit>()

    val labels: List<String> get() = files.mapNotNull { it.label }

    // Random rather than counted: through a hub every client watching the pane hears the answer,
    // and the id is how this one tells its own from theirs. Labelled in muffit's own style rather
    // than Claude's `[Image #n]`, so a label is never taken for a token the pane drew.
    fun add(
        name: String,
        bytes: ByteArray,
        state: PendingFile.State = PendingFile.State.Sending,
        inline: Boolean = false,
        picture: Boolean = false,
    ): PendingFile {
        val label = if (inline || state is PendingFile.State.Refused) {
            null
        } else {
            "[${if (picture) "image" else "file"} ${++labelled}: $name]"
        }
        val file = PendingFile(Random.nextLong().toULong().toString(36), name, bytes, state, inline, label)
        files += file
        label?.let { edits += BoxEdit.Insert(it) }
        return file
    }

    // What the node typed into the pane for a file placed there, which then leaves; null for any
    // other answer.
    fun answered(msg: ServerMsg.Pasted): String? {
        val id = msg.id ?: return null
        if (msg.path != null && files.any { it.id == id && it.inline }) {
            remove(id)
            return msg.text
        }
        val state = when {
            msg.refused != null -> PendingFile.State.Refused(msg.refused)
            msg.path != null -> PendingFile.State.Ready(msg.path, msg.text ?: "${msg.path} ")
            else -> return null
        }
        move(id, state)
        return null
    }

    // A pane error while a file is on its way is the only word there will be about it: a hub that
    // could not reach the peer, or a device that may not type, answers with no id.
    fun refuseSending(reason: String) {
        files.filter { it.state == PendingFile.State.Sending }.forEach { move(it.id, PendingFile.State.Refused(reason)) }
    }

    fun refuse(id: String, reason: String) = move(id, PendingFile.State.Refused(reason))

    fun remove(id: String) {
        files.filter { it.id == id }.forEach { file -> file.label?.let { edits += BoxEdit.Remove(it) } }
        files.removeAll { it.id == id }
    }

    // When a placed file began waiting for its answer: the first time anything asked, kept so a
    // composition that left and came back neither restarts the wait nor loses it.
    fun waiting(id: String, now: Double): Double? {
        val at = files.indexOfFirst { it.id == id && it.inline && it.state == PendingFile.State.Sending }
        if (at < 0) return null
        return files[at].since ?: now.also { files[at] = files[at].with(files[at].state, since = it) }
    }

    fun unanswered(id: String) {
        files.removeAll { it.id == id && it.inline && it.state == PendingFile.State.Sending }
    }

    // A label the operator deleted from the box takes its file with it.
    fun gone(label: String) {
        files.removeAll { it.label == label }
    }

    // The reply, with each label in it become what the node said to type for its file, and those
    // files then leave. A refusal stays: it is the only report that a file never arrived, and a send
    // that had nothing to do with it is no reason to sweep it away unread.
    fun take(words: String): String {
        val ready = files.filter { it.state is PendingFile.State.Ready }
        files.removeAll(ready)
        return ready.fold(words) { said, file ->
            val label = file.label ?: return@fold said
            said.replace(label, (file.state as PendingFile.State.Ready).text.trimEnd())
        }.trimEnd()
    }

    private fun move(id: String, state: PendingFile.State) {
        val at = files.indexOfFirst { it.id == id }
        if (at < 0) return
        val file = files[at]
        if (state is PendingFile.State.Refused) file.label?.let { edits += BoxEdit.Remove(it) }
        files[at] = file.with(state)
    }
}
