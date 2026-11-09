package dev.kampr.conversation

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.GlyphTarget
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.Track
import kotlinx.coroutines.delay

// How often a sound is asked how it is doing. Its answers move a clock that shows seconds and put a
// button back at the end of a clip, so a quarter second is short enough that nobody catches it.
private const val SOUND_POLL_MILLIS = 250L

fun clockOf(ms: Long): String {
    val seconds = (ms / 1000).coerceAtLeast(0)
    val hours = seconds / 3600
    val minutes = seconds / 60 % 60
    val secs = (seconds % 60).toString().padStart(2, '0')
    return if (hours > 0) "$hours:${minutes.toString().padStart(2, '0')}:$secs" else "$minutes:$secs"
}

// A recording streaming from the node: play and pause, where it is, and a track to move through it.
//
// The decoder is opened here and let go of when this leaves the composition. On two of the three
// targets a [Voice] holds a hardware line, and anything that outlived the row drawing its button
// would leave it running with nothing left to stop it.
@Composable
fun SoundPlayer(
    name: String,
    url: String,
    mime: String?,
    again: () -> Unit,
    modifier: Modifier = Modifier,
    startAtOnce: Boolean = false,
    trailing: @Composable RowScope.() -> Unit = {},
) {
    val tokens = Kampr.tokens
    val voices = LocalVoices.current
    val voice = remember(url, voices) { voices.stream(url, mime) }
    DisposableEffect(voice) { onDispose { voice.release() } }
    var sounding by remember(voice) { mutableStateOf(false) }
    var failed by remember(voice) { mutableStateOf(false) }
    var position by remember(voice) { mutableLongStateOf(0L) }
    var duration by remember(voice) { mutableStateOf<Long?>(null) }
    var dragged by remember(voice) { mutableStateOf<Float?>(null) }
    // Outlives the voice: `again` hands this player a new URL and so a new voice, and the reader
    // who pressed it is still listening to the same recording.
    var resumeAt by remember { mutableStateOf<Long?>(null) }
    LaunchedEffect(voice) {
        val resume = resumeAt
        resumeAt = null
        if (resume != null) voice.seek(resume)
        if (startAtOnce || resume != null) voice.play()
        while (true) {
            sounding = voice.playing
            failed = voice.failed
            position = voice.positionMs
            duration = voice.durationMs?.takeIf { it > 0 }
            delay(SOUND_POLL_MILLIS)
        }
    }
    val length = duration

    DisableSelection {
        Column(modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp)) {
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(9.dp),
            ) {
                if (failed) {
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
                if (length != null) {
                    val shown = dragged?.let { (it * length).toLong() } ?: position
                    KText("${clockOf(shown)} / ${clockOf(length)}", tokens.type.micro, tokens.color.mute, maxLines = 1)
                }
                trailing()
            }
            if (length != null && !failed) {
                Track(
                    dragged ?: (position.toFloat() / length).coerceIn(0f, 1f),
                    { dragged = it },
                    "Position in $name",
                    "${clockOf(position)} of ${clockOf(length)}",
                    onSettled = {
                        dragged?.let { target ->
                            val at = (target * length).toLong()
                            voice.seek(at)
                            position = at
                        }
                        dragged = null
                    },
                    jumps = true,
                )
            }
            // Said only where the reader was offered a press that then could not be honoured: the
            // type was one this device claims and the node sent something else, or the ticket died
            // under it — with the node that minted it, or of age.
            if (failed) {
                KText(
                    "it would not play — try again, or save it instead",
                    tokens.type.micro,
                    tokens.color.blocked,
                    Modifier.fillMaxWidth().padding(vertical = 2.dp),
                    maxLines = 2,
                )
                QuietAction(
                    "Try again",
                    {
                        resumeAt = position
                        again()
                    },
                    Modifier.fillMaxWidth(),
                    label = "Try again, $name",
                )
            }
        }
    }
}
