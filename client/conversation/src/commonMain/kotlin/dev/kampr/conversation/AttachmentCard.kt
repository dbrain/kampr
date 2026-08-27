package dev.kampr.conversation

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
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
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.wire.Attachment
import kotlinx.coroutines.launch

private val THUMBNAIL = 64.dp

@Composable
fun AttachmentCard(att: Attachment, attachments: AttachmentStore, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val io = LocalPaneIo.current
    val scope = rememberCoroutineScope()
    val state = attachments.state(att.id)
    val headline = headlineOf(att)
    val offer = offerFor(att)

    if (state is AttachmentState.Shown) {
        // A thumbnail and its name, not the picture at full width. A screenshot of a 292-column
        // pane scaled into a phone's column is unreadable *and* taller than the column, so it cost
        // the reader a screen of scrolling to pass something they could not read either way. The
        // name is what tells them whether it is worth opening, which is the question the picture
        // itself could not answer at that size.
        Surface(modifier.fillMaxWidth(), background = tokens.color.raise, radius = tokens.radii.md) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .touchable(LANDSCAPE_TOUCH)
                    .action("Open $headline", { attachments.view(att) })
                    .padding(10.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(11.dp),
            ) {
                Image(
                    bitmap = state.image,
                    contentDescription = null,
                    modifier = Modifier
                        .size(THUMBNAIL)
                        .clip(RoundedCornerShape(tokens.radii.sm)),
                    contentScale = ContentScale.Crop,
                )
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    KText(headline, tokens.type.meta, tokens.color.text)
                    detailOf(att)?.let { KText(it, tokens.type.micro, tokens.color.mute) }
                }
                IconGlyph(ConversationIcons.image, 14.dp, tokens.color.mute)
            }
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
