package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.touchable

// Folded until asked for, and nothing in it is fetched until a row or the tiles are pressed: a
// reply that names forty files is one line under it. Tiles are thumbnails, so they wait for their
// own press rather than arriving with the list.
@Composable
fun FilesTray(
    paths: List<String>,
    open: Boolean,
    onToggle: () -> Unit,
    attachments: AttachmentStore,
    modifier: Modifier = Modifier,
) {
    if (LocalPaneIo.current.readOnly || paths.isEmpty()) return
    val tokens = Kampr.tokens
    val palette = rememberConversationPalette()
    val shots = remember(paths) { paths.map(::fileTarget).filter { offerFor(it) == AttachmentOffer.Image } }
    var tiles by remember(paths) { mutableStateOf(false) }
    val count = if (paths.size == 1) "1 file" else "${paths.size} files"

    Surface(modifier.fitContent(), background = tokens.color.raise, radius = tokens.radii.md) {
        Column {
            Header(
                if (open) "Hide the $count this reply names" else "Show the $count this reply names",
                "$count in this reply",
                open,
                onToggle,
            )
            if (open) {
                if (shots.size > 1) {
                    Box(Modifier.fillMaxWidth().height(1.dp).background(palette.rule))
                    if (tiles) {
                        Gallery(shots, attachments, Modifier.fillMaxWidth().padding(10.dp))
                    } else {
                        Header("Show the ${shots.size} pictures as tiles", "${shots.size} pictures as tiles", false) {
                            tiles = true
                        }
                    }
                }
                for (path in paths) {
                    Box(Modifier.fillMaxWidth().height(1.dp).background(palette.rule))
                    FileAffordance(path, attachments)
                }
            }
        }
    }
}

@Composable
private fun Header(action: String, label: String, open: Boolean, onPress: () -> Unit) {
    val tokens = Kampr.tokens
    DisableSelection {
        Row(
            Modifier
                .fillMaxWidth()
                .touchable(LANDSCAPE_TOUCH)
                .action(action, onPress, selected = open)
                .padding(horizontal = 12.dp, vertical = 9.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(9.dp),
        ) {
            IconGlyph(ConversationIcons.file, 13.dp, tokens.color.dim)
            KText(label, tokens.type.meta, tokens.color.dim, Modifier.weight(1f), maxLines = 1)
            IconGlyph(if (open) ConversationIcons.chevronUp else ConversationIcons.chevronDown, 12.dp, tokens.color.mute)
        }
    }
}
