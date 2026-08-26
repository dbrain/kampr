package dev.kampr.conversation

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.announce
import dev.kampr.shared.wire.Attachment
import kotlinx.coroutines.launch

// Half a phone screen. A screenshot of a wide terminal is taller than the column it lands in once
// it is scaled to the width, and a reader who has to scroll past one picture to reach the next
// message has lost the transcript.
private val TALLEST_IMAGE = 460.dp

@Composable
fun AttachmentCard(att: Attachment, attachments: AttachmentStore, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val io = LocalPaneIo.current
    val scope = rememberCoroutineScope()
    val state = attachments.state(att.id)
    val headline = headlineOf(att)
    val offer = offerFor(att)

    if (state is AttachmentState.Shown) {
        val ratio = (state.image.width.toFloat() / state.image.height.toFloat())
            .takeIf { it.isFinite() && it > 0f } ?: 1f
        // Measured rather than declared: `aspectRatio` given a fixed width will hand back a height
        // past the `heightIn` above it, and a 900x1400 screenshot then runs 672 dp down a column
        // it was supposed to fit inside.
        BoxWithConstraints(modifier.fillMaxWidth()) {
            val tall = minOf(maxWidth / ratio, TALLEST_IMAGE)
            Image(
                bitmap = state.image,
                contentDescription = listOfNotNull(headline, detailOf(att)).joinToString(", "),
                modifier = Modifier
                    .size(tall * ratio, tall)
                    .clip(RoundedCornerShape(tokens.radii.md)),
                contentScale = ContentScale.Fit,
            )
        }
        return
    }

    Surface(modifier.fillMaxWidth(), radius = tokens.radii.md) {
        Column(
            Modifier.fillMaxWidth().padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(9.dp),
        ) {
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(9.dp),
            ) {
                IconGlyph(
                    when (offer) {
                        AttachmentOffer.Image -> ConversationIcons.image
                        AttachmentOffer.Video -> ConversationIcons.film
                        AttachmentOffer.File -> ConversationIcons.download
                    },
                    15.dp,
                    tokens.color.dim,
                )
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    KText(headline, tokens.type.meta, tokens.color.text)
                    detailOf(att)?.let { KText(it, tokens.type.micro, tokens.color.mute) }
                }
            }
            when (state) {
                AttachmentState.Fetching -> KText(
                    "fetching",
                    tokens.type.micro,
                    tokens.color.working,
                    Modifier.announce("Fetching $headline"),
                )

                is AttachmentState.Saved -> KText(
                    "saved to ${state.where}",
                    tokens.type.micro,
                    tokens.color.done,
                    Modifier.fillMaxWidth().announce("Saved to ${state.where}"),
                    maxLines = 2,
                )

                is AttachmentState.Failed -> {
                    KText(
                        state.reason,
                        tokens.type.caption,
                        tokens.color.blocked,
                        Modifier.fillMaxWidth().announce(state.reason),
                        maxLines = 3,
                    )
                    // The node's own reason for the failure stays selectable above this; the
                    // press is chrome, and a caption pasted into a bug report is noise.
                    DisableSelection {
                        QuietAction(
                            "Try again",
                            { scope.launch { attachments.open(io, att) } },
                            Modifier.fillMaxWidth(),
                            label = "Try again, $headline",
                        )
                    }
                }

                else -> DisableSelection {
                    QuietAction(
                        offer.label,
                        { scope.launch { attachments.open(io, att) } },
                        Modifier.fillMaxWidth(),
                        label = "${offer.label}, $headline",
                    )
                }
            }
        }
    }
}
