package dev.kampr.conversation

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.wire.Attachment

// The transcript is a lazy list, and a row that measures against the picture inside it rather than
// against a ceiling is a scroll bug: a 1400 px screenshot would push every message after it a
// screen and a half down. Fitted inside this, so a tall picture is small and a wide one is wide.
private val TALLEST = 260.dp

// A picture the operator handed over, shown where they handed it over rather than named there. The
// node writes the bytes on the pane's own machine and types the path in, so what lands in the
// transcript is a literal path — and the picture is on that machine, one authorised fetch away at
// an id built from the path.
//
// **Fetched without being asked**, which the press-to-open card beside it deliberately is not: this
// is the operator's own message and the picture *is* the message. Bounded on both counts — at most
// two per paragraph of it, and never taller than [TALLEST].
//
// Everything short of the picture falls back to the path the prose above already shows. A
// read-only device is offered nothing at all and asks for nothing, because the whole security
// argument for a path-shaped id is that a device that may type into a terminal can already `cat`
// the file and a device that may not is exactly the one that must not reach `~/.ssh/id_rsa`. A
// fetch that fails says why in the node's own words rather than leaving an empty frame (#233) —
// and it will fail, because the path resolves on the pane's machine at read time and the file is
// only there while it is still there.
//
// Pressing it opens [ImageViewer], which is where the panning and the zooming live. Not here: this
// row is inside a vertically scrolling list, and a drag handler on it would take the scroll.
@Composable
fun InlinePicture(att: Attachment, attachments: AttachmentStore, modifier: Modifier = Modifier) {
    val io = LocalPaneIo.current
    if (io.readOnly) return
    val tokens = Kampr.tokens
    val headline = headlineOf(att)
    LaunchedEffect(att.id) {
        if (attachments.state(att.id) is AttachmentState.Idle) attachments.open(io, att)
    }
    when (val state = attachments.state(att.id)) {
        is AttachmentState.Shown -> Image(
            bitmap = state.image,
            contentDescription = null,
            modifier = modifier
                .heightIn(max = TALLEST)
                .clip(RoundedCornerShape(tokens.radii.md))
                .action("Open $headline", { attachments.view(att) }),
            contentScale = ContentScale.Fit,
            alignment = Alignment.CenterStart,
        )

        AttachmentState.Fetching -> KText(
            "fetching $headline",
            tokens.type.micro,
            tokens.color.working,
            modifier.announce("Fetching $headline"),
        )

        is AttachmentState.Failed -> KText(
            state.reason,
            tokens.type.micro,
            tokens.color.blocked,
            modifier.announce(state.reason),
            maxLines = 3,
        )

        // Bytes with a picture's extension that no picture decoder would take. The store hands
        // those to the device rather than dropping them, and saying so is the difference between a
        // file that quietly arrived and a press that did nothing.
        is AttachmentState.Saved -> KText(
            "saved to ${state.where}",
            tokens.type.micro,
            tokens.color.done,
            modifier.announce("Saved to ${state.where}"),
            maxLines = 2,
        )

        else -> Unit
    }
}
