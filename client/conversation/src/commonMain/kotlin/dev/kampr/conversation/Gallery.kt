package dev.kampr.conversation

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.action
import dev.kampr.shared.wire.Attachment
import kotlinx.coroutines.launch

private val TILE = 112.dp

// A tile is a thumbnail and the file's name, and pressing it opens the full picture. The full
// picture is fetched again on the press rather than kept from the thumbnail's fetch: the store
// holds four of those, and a gallery of eight that held its own would be pushing them out of each
// other and fetching again on every return to the pane.
//
// A read-only device is offered nothing, for the reason [InlinePicture] gives.
@Composable
fun Gallery(shots: List<Attachment>, attachments: AttachmentStore, modifier: Modifier = Modifier) {
    val io = LocalPaneIo.current
    if (io.readOnly) return
    FlowRow(modifier, horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        for (shot in shots) Tile(shot, attachments)
    }
}

@Composable
private fun Tile(att: Attachment, attachments: AttachmentStore) {
    val io = LocalPaneIo.current
    val tokens = Kampr.tokens
    val scope = rememberCoroutineScope()
    val headline = headlineOf(att)
    val thumb = attachments.thumbnail(att.id)
    // Watched rather than keyed on, so a tile whose thumbnail the store let go of asks again: a key
    // on the state would cancel the fetch the moment it marked itself as fetching.
    LaunchedEffect(att.id) {
        snapshotFlow { attachments.thumbnail(att.id) is Thumbnail.Idle }.collect { idle ->
            if (idle) attachments.fetchThumbnail(io, att)
        }
    }
    val full = attachments.state(att.id)
    val label = when {
        full is AttachmentState.Failed -> full.reason
        full is AttachmentState.Fetching -> "opening"
        thumb is Thumbnail.Failed -> thumb.reason
        else -> null
    }
    val shape = RoundedCornerShape(tokens.radii.md)
    Column(
        Modifier
            .width(TILE)
            .action(
                when (thumb) {
                    is Thumbnail.Shown -> "Open $headline"
                    is Thumbnail.Failed -> "Try $headline again"
                    else -> "Fetching $headline"
                },
                {
                    scope.launch {
                        when {
                            thumb is Thumbnail.Failed -> attachments.fetchThumbnail(io, att)
                            full is AttachmentState.Shown -> attachments.view(att)
                            else -> attachments.reveal(io, att)
                        }
                    }
                },
                shape = shape,
            ),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Box(Modifier.size(TILE).clip(shape).background(tokens.color.raise), contentAlignment = Alignment.Center) {
            when (thumb) {
                is Thumbnail.Shown -> Image(thumb.image, null, Modifier.fillMaxSize(), alignment = Alignment.TopCenter, contentScale = ContentScale.Crop)

                else -> KText(
                    if (thumb is Thumbnail.Failed) "failed" else "fetching",
                    tokens.type.micro,
                    if (thumb is Thumbnail.Failed) tokens.color.blocked else tokens.color.working,
                )
            }
        }
        KText(headline, tokens.type.micro, tokens.color.mute, Modifier.padding(horizontal = 2.dp), maxLines = 1)
        if (label != null) KText(label, tokens.type.micro, tokens.color.blocked, maxLines = 3)
    }
}
