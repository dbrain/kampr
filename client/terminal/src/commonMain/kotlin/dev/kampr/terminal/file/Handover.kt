package dev.kampr.terminal.file

import dev.kampr.shared.model.PaneState
import dev.kampr.shared.platform.PickedFile
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlin.io.encoding.Base64

// Where a file the operator handed over has got to. The node writes the bytes on the pane's own
// machine and types the path in, so "sent" is the whole of the success — there is no upload to
// watch — and a refusal comes back as an error naming this pane.
sealed interface Handover {
    data object Idle : Handover
    data class Going(val name: String) : Handover
    data class Sent(val name: String) : Handover
    data class Refused(val reason: String) : Handover
}

// The ceiling the node applies, applied here as well. Sending eight megabytes up a phone link to
// be refused at the other end is a minute of somebody's tethering spent on a certain no.
const val MOST_BYTES_HANDED_OVER = 8 * 1024 * 1024

fun handoverName(picked: PickedFile): String = picked.name?.takeIf { it.isNotBlank() } ?: "the file"

// The node answers a paste it will not take with an error naming this pane — too large, not
// base64, nowhere to write — and that error is deliberately quiet everywhere else. The refusal is
// cleared before the bytes go, so what lands next is the answer to this.
fun handoverAfter(handover: Handover, refusal: String?): Handover =
    if (refusal != null && handover is Handover.Sent) Handover.Refused(refusal) else handover

// Off the main thread, because base64 of eight megabytes is eight megabytes of work and this
// surface is repainting a terminal grid while it happens. That wait is what `Going` is for.
suspend fun handoverOf(pane: PaneState, io: PaneIo, picked: PickedFile): Handover {
    val name = handoverName(picked)
    if (picked.bytes.size > MOST_BYTES_HANDED_OVER) {
        return Handover.Refused("$name is larger than the 8 MiB a node will take.")
    }
    val b64 = withContext(Dispatchers.Default) { Base64.encode(picked.bytes) }
    pane.clearRefusal()
    io.send(ClientMsg.Paste(pane.id, b64, picked.name))
    return Handover.Sent(name)
}
