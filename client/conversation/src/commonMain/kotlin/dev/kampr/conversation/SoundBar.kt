package dev.kampr.conversation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.wire.Attachment
import kotlinx.coroutines.launch

// A recording in a transcript, with the two things a reader can do to it: hear it, and keep it.
// It starts as soon as it is drawn, because the press that drew it was the reader asking to hear it.
@Composable
fun SoundBar(
    att: Attachment,
    sound: AttachmentState.Sound,
    attachments: AttachmentStore,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val io = LocalPaneIo.current
    val scope = rememberCoroutineScope()
    val name = att.name ?: "audio"
    val saved = attachments.saved(att.id)
    SoundPlayer(name, sound.url, sound.mime, { scope.launch { attachments.restream(io, att) } }, modifier, startAtOnce = true) {
        if (saved != null) {
            KText("saved to $saved", tokens.type.micro, tokens.color.done, maxLines = 2)
        } else {
            GlyphTarget(
                ConversationIcons.download,
                "Save $name",
                tokens.color.mute,
                { scope.launch { attachments.save(io, att) } },
                target = LANDSCAPE_TOUCH,
                glyph = 13.dp,
            )
        }
    }
}
