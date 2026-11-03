package dev.kampr.conversation

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.files.HeldBytes
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.PendingFile
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.platform.PickedFile
import dev.kampr.shared.platform.decodeImage
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.recordPlace
import dev.kampr.shared.wire.Attachment
import dev.kampr.shared.wire.ClientMsg
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlin.io.encoding.Base64

// The ceiling the node applies, applied here as well. Sending eight megabytes up a phone link to be
// refused at the other end is a minute of somebody's tethering spent on a certain no.
internal const val MOST_BYTES_HANDED_OVER = 8 * 1024 * 1024

private val CHIP = 40.dp
private val THUMB = 30.dp

// A box that is the pane's line takes the file in the pane, at the reply box's caret: the node
// types it there, a harness that draws a picture draws its token (#620), and it comes back as the
// line this box mirrors. Any other box holds it as a label at its caret, and the node writes the
// bytes and types nothing (`typed = false`): a path typed now would sit on the harness's input line
// and come straight back as the desk line over this box. `live` is a box that may type.
internal fun attach(pane: PaneState, io: PaneIo, picked: PickedFile, live: Boolean = false) {
    val name = picked.name?.takeIf { it.isNotBlank() } ?: "the file"
    if (picked.bytes.size > MOST_BYTES_HANDED_OVER) {
        pane.files.add(name, ByteArray(0), PendingFile.State.Refused("larger than the 8 MiB a node will take"))
        return
    }
    pane.clearRefusal()
    val undelivered = pane.undelivered
    val placing = pane.mirror.place(pane.draft, pane.draftCaret, live)
    val picture = picked.mime?.startsWith("image/") == true
    val file = pane.files.add(name, picked.bytes, inline = placing != null, picture = picture)
    placing?.forEach { io.send(ClientMsg.InputText(pane.id, it)) }
    io.send(ClientMsg.Paste(pane.id, Base64.encode(picked.bytes), picked.name, typed = placing != null, id = file.id))
    // Dropped rather than queued, as a keystroke is, so a chip still saying "sending" would be a lie.
    if (pane.undelivered > undelivered) pane.files.refuse(file.id, "not sent, there is no connection to the node")
}

// The file viewer reads a record from the bytes this device holds before it asks the node, so a
// chip opens what was picked without a round trip and without an id the node would recognise.
internal fun openHeld(file: PendingFile, paneId: String, held: HeldBytes?, open: (FilesPlace) -> Unit) {
    val att = Attachment(id = "held:${file.id}", bytes = file.bytes.size.toLong(), name = file.name)
    held?.put(paneId, att.id, AttachmentBytes.Ok(file.bytes, null))
    open(recordPlace(paneId, att))
}

@Composable
internal fun FileChips(files: List<PendingFile>, onOpen: (PendingFile) -> Unit, onRemove: (PendingFile) -> Unit) {
    if (files.isEmpty()) return
    val tokens = Kampr.tokens
    Column(Modifier.fillMaxWidth().padding(start = 16.dp, end = 12.dp, top = 8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            for (file in files) FileChip(file, onOpen, onRemove)
        }
        for (file in files) {
            val refused = file.state as? PendingFile.State.Refused ?: continue
            val words = "${file.name}: ${refused.reason}"
            KText(words, tokens.type.micro, tokens.color.blocked, Modifier.announce(words), maxLines = 3)
        }
    }
}

@Composable
private fun FileChip(file: PendingFile, onOpen: (PendingFile) -> Unit, onRemove: (PendingFile) -> Unit) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    val state = file.state
    val tone = when (state) {
        PendingFile.State.Sending -> tokens.color.working
        is PendingFile.State.Ready -> tokens.color.text
        is PendingFile.State.Refused -> tokens.color.blocked
    }
    val thumb by produceState<ImageBitmap?>(null, file.id) {
        value = withContext(Dispatchers.Default) { decodeImage(file.bytes)?.within(THUMBNAIL_PX) }
    }
    Row(
        Modifier.height(CHIP).clip(shape).background(tokens.color.surface).edge(tokens.card, shape),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val face = when (state) {
            is PendingFile.State.Ready -> Modifier.action("Open ${file.name}", { onOpen(file) }, shape)
            PendingFile.State.Sending -> Modifier.named("Sending ${file.name}")
            is PendingFile.State.Refused -> Modifier.named("${file.name} was refused")
        }
        Row(face.padding(start = 5.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Box(Modifier.size(THUMB).clip(RoundedCornerShape(tokens.radii.sm)).background(tokens.color.raise), contentAlignment = Alignment.Center) {
                when (val image = thumb) {
                    null -> IconGlyph(ConversationIcons.file, 15.dp, tone)
                    else -> Image(image, null, Modifier.size(THUMB), contentScale = ContentScale.Crop)
                }
            }
            Column {
                KText(file.name, tokens.type.micro, tone, Modifier.widthIn(max = 140.dp), maxLines = 1)
                if (state == PendingFile.State.Sending) KText("sending", tokens.type.micro, tokens.color.working)
            }
        }
        GlyphTarget(ConversationIcons.close, "Remove ${file.name}", tokens.color.dim, { onRemove(file) }, target = CHIP, glyph = 11.dp)
    }
}
