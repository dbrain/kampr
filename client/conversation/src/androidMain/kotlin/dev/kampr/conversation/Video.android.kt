package dev.kampr.conversation

import android.net.Uri
import android.widget.MediaController
import android.widget.VideoView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.viewinterop.AndroidView

// The containers Android's media framework is documented to read on every device from the minimum
// SDK up. QuickTime is not one of them, whatever a particular phone happens to manage.
private val PLAYABLE = setOf("video/mp4", "video/webm", "video/3gpp")

actual val deviceFilms: Films = object : Films {
    override fun canPlay(mime: String?): Boolean = videoType(mime) in PLAYABLE

    // `VideoView` streams the URL itself, ranges and all, and `MediaController` is the platform's
    // own play, pause and seek bar over it.
    @Composable
    override fun Screen(url: String, name: String, onFailed: () -> Unit, modifier: Modifier) {
        val failed by rememberUpdatedState(onFailed)
        AndroidView(
            factory = { context ->
                VideoView(context).apply {
                    setMediaController(MediaController(context).also { it.setAnchorView(this) })
                    setVideoURI(Uri.parse(url))
                    setOnPreparedListener { start() }
                    // Handled, so the platform's own "Can't play this video" dialog stays away and
                    // the viewer's note and its Try again stand in for it.
                    setOnErrorListener { _, _, _ ->
                        failed()
                        true
                    }
                }
            },
            modifier = modifier.semantics { contentDescription = "Video $name" },
            onRelease = { it.stopPlayback() },
        )
    }
}
