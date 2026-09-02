package dev.kampr.conversation

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.diffAttachmentId
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.wire.Attachment
import kotlinx.coroutines.launch

// The node's own list, and it is short on purpose: a type that is not on it is served as a
// download, which is the safe answer for `text/html` and for the scriptable document
// `image/svg+xml` names. Guessing a kind the route will not serve inline only mislabels a button.
private val PICTURE = setOf("png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "ico")

// The same table the node's sniffer mints from, keyed by the only thing a path offers. It is a
// guess about a name, and it decides exactly one thing: whether this device is asked, before a
// byte is fetched, if it could play the file at all. What reaches the player is the type the
// **node** derived from the bytes, never this.
private val SOUND = mapOf(
    "wav" to "audio/wav",
    "wave" to "audio/wav",
    "mp3" to "audio/mpeg",
    "m4a" to "audio/mp4",
    "ogg" to "audio/ogg",
    "oga" to "audio/ogg",
    "opus" to "audio/ogg",
    "flac" to "audio/flac",
    "aif" to "audio/aiff",
    "aiff" to "audio/aiff",
)

// Types whose bytes are never words. Everything not named here stays `text` on purpose: the file
// viewer reads source, logs and notes, and that is what most of the paths in a transcript are, so
// the default has to be the one that reads them rather than the one that saves them.
private val OPAQUE = setOf(
    "zip", "tar", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar",
    "pdf", "mp4", "m4v", "mov", "mkv", "webm", "avi",
    "woff", "woff2", "ttf", "otf",
    "wasm", "exe", "dll", "so", "dylib", "bin", "iso", "jar", "apk", "aab", "class",
    "db", "sqlite", "sqlite3",
)

fun fileName(path: String): String = path.trimEnd('/').substringAfterLast('/').ifEmpty { path }

private fun extensionOf(path: String): String =
    fileName(path).substringAfterLast('.', "").lowercase()

// What a name says the file is, for the question [canPlayAudio] answers. Null for everything that
// is not a recording, which is what makes a path in a sentence a row rather than a guess.
fun soundType(path: String?): String? = path?.let { SOUND[extensionOf(it)] }

// A path the node derived, made fetchable. The id is built here rather than handed over by the
// node: an id minted from a record names a record, and a record that has been rewritten under it
// stops resolving — the file is still on disk, and this is the form that says so.
//
// No `mime`: [AttachmentStore] prefers a header's own type over what came back, and what came back
// is the node's answer off the bytes. A guess from an extension put here would win over it and be
// wrong about every file somebody misnamed.
fun fileTarget(path: String): Attachment = Attachment(
    id = fileAttachmentId(path),
    kind = when (extensionOf(path)) {
        in PICTURE -> "image"
        in SOUND -> "audio"
        in OPAQUE -> "file"
        else -> "text"
    },
    name = fileName(path),
)

// The same path, asked about rather than read.
fun diffTarget(path: String): Attachment = Attachment(
    id = diffAttachmentId(path),
    kind = "text",
    mime = "text/plain",
    name = fileName(path),
)

// One press: fetch, then show what came back. Absent for a read-only device, which the route
// refuses outright — the whole security argument for this form of id is that a device that may
// type into a terminal can already `cat` the file, and a device that may not is exactly the one
// that must not reach `~/.ssh/id_rsa`.
@Composable
fun FileAffordance(path: String, attachments: AttachmentStore, modifier: Modifier = Modifier) {
    val io = LocalPaneIo.current
    if (io.readOnly) return
    val tokens = Kampr.tokens
    val scope = rememberCoroutineScope()
    val att = remember(path) { fileTarget(path) }
    val state = attachments.state(att.id)
    val name = att.name ?: path

    // A recording that arrived becomes the player in place. It is not sent to a viewer over the
    // whole pane the way a picture and a file are: there is nothing to look at, and a sound the
    // reader started has to keep running while they go on reading the transcript under it.
    if (state is AttachmentState.Sound) {
        SoundBar(att, state, attachments, modifier)
        return
    }

    val audible = offerOn(att, LocalVoices.current, path) == AttachmentOffer.Audio
    val verb = if (audible) "Play" else "Open"
    val (word, tone, press) = when (state) {
        AttachmentState.Fetching -> Triple("fetching", tokens.color.working, null)
        is AttachmentState.Failed -> Triple(state.reason, tokens.color.blocked, {
            scope.launch { attachments.reveal(io, att) }
            Unit
        })
        is AttachmentState.Saved -> Triple("saved to ${state.where}", tokens.color.done, null)
        else -> Triple(verb.lowercase(), tokens.color.dim, {
            scope.launch { attachments.reveal(io, att) }
            Unit
        })
    }

    DisableSelection {
        Row(
            modifier
                .fillMaxWidth()
                .let { base ->
                    if (press == null) base.announce("$name, $word")
                    else base.touchable(LANDSCAPE_TOUCH).action("$verb $name", press)
                }
                .padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(9.dp),
        ) {
            IconGlyph(
                if (audible) ConversationIcons.sound else ConversationIcons.file,
                13.dp,
                tokens.color.dim,
            )
            KText(name, tokens.type.micro, tokens.color.dim, Modifier.weight(1f), maxLines = 1)
            KText(word, tokens.type.micro, tone, maxLines = 2)
            if (press != null) IconGlyph(KamprIcons.chevronRight, 11.dp, tokens.color.mute)
        }
    }
}
