package dev.kampr.conversation

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.wire.Attachment
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

// How often a running sound is asked whether it still is. The only thing the answer changes is the
// glyph on the button when a clip reaches its end, so the interval only has to be short enough
// that nobody catches it — and it runs only while a sound is actually playing.
private const val SOUND_POLL_MILLIS = 250L

// A recording, with the two things a reader can do to it: hear it, and keep it.
//
// The decoder is opened here rather than in the store, and let go of when this row leaves the
// composition. On two of the three targets a [Voice] holds a hardware line, and a store that
// evicted one on its byte budget would leave it running with nothing left to stop it.
@Composable
fun SoundBar(
    att: Attachment,
    sound: AttachmentState.Sound,
    attachments: AttachmentStore,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val scope = rememberCoroutineScope()
    val name = att.name ?: "audio"
    val voices = LocalVoices.current
    val voice = remember(sound, voices) { voices.open(sound.bytes, sound.mime) }
    DisposableEffect(voice) { onDispose { voice?.release() } }
    var sounding by remember(voice) { mutableStateOf(false) }
    LaunchedEffect(sounding) {
        while (sounding) {
            delay(SOUND_POLL_MILLIS)
            if (voice?.playing != true) sounding = false
        }
    }
    val saved = attachments.saved(att.id)

    DisableSelection {
        Row(
            modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(9.dp),
        ) {
            if (voice == null) {
                IconGlyph(ConversationIcons.sound, 13.dp, tokens.color.dim)
            } else {
                GlyphTarget(
                    if (sounding) ConversationIcons.pause else ConversationIcons.play,
                    if (sounding) "Pause $name" else "Play $name",
                    tokens.color.text,
                    {
                        if (sounding) voice.pause() else voice.play()
                        sounding = !sounding
                    },
                    target = LANDSCAPE_TOUCH,
                    glyph = 13.dp,
                )
            }
            KText(name, tokens.type.micro, tokens.color.dim, Modifier.weight(1f), maxLines = 1)
            when {
                saved != null -> KText("saved to $saved", tokens.type.micro, tokens.color.done, maxLines = 2)
                // The bytes are already in hand and a second press would be a second authorised
                // round trip for bytes that never left.
                else -> GlyphTarget(
                    ConversationIcons.download,
                    "Save $name",
                    tokens.color.mute,
                    { scope.launch { attachments.save(att) } },
                    target = LANDSCAPE_TOUCH,
                    glyph = 13.dp,
                )
            }
        }
        // Said only where the reader was offered a press that then could not be honoured: the type
        // was one this device claims, and the bytes behind it were not.
        if (voice == null) {
            KText(
                "this device would not play it — save it instead",
                tokens.type.micro,
                tokens.color.blocked,
                Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 2.dp),
                maxLines = 2,
            )
        }
    }
}
