package dev.kampr.conversation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.HtmlElementView
import kotlinx.browser.document
import org.w3c.dom.HTMLVideoElement

private val answered = HashMap<String, Boolean>()

// The browser is the only honest answer about what the browser will play: H.264 is missing from
// some Linux Chromium builds, and QuickTime plays in Safari and almost nowhere else.
private fun jsCanPlayVideo(mime: String): Boolean = js(
    """
    (function () {
      try { return document.createElement('video').canPlayType(mime) !== ''; } catch (e) { return false; }
    })()
    """,
)

actual val deviceFilms: Films = object : Films {
    override fun canPlay(mime: String?): Boolean {
        val type = videoType(mime) ?: return false
        return answered.getOrPut(type) { runCatching { jsCanPlayVideo(type) }.getOrDefault(false) }
    }

    // A real `<video>` element laid over the canvas where this composable sits, so the browser
    // draws it, decodes it and gives it the controls a reader expects, fullscreen included.
    // Emptying `src` and calling `load` on release is what makes the element drop its connection
    // and its buffer; a detached element with its source set keeps both.
    @OptIn(ExperimentalComposeUiApi::class)
    @Composable
    override fun Screen(url: String, name: String, onFailed: () -> Unit, modifier: Modifier) {
        val failed by rememberUpdatedState(onFailed)
        HtmlElementView(
            factory = {
                (document.createElement("video") as HTMLVideoElement).apply {
                    controls = true
                    preload = "metadata"
                    setAttribute("playsinline", "")
                    setAttribute("aria-label", "Video $name")
                    style.width = "100%"
                    style.height = "100%"
                    style.backgroundColor = "black"
                    addEventListener("error", { failed() })
                    src = url
                }
            },
            modifier = modifier,
            onRelease = { video ->
                video.pause()
                video.removeAttribute("src")
                video.load()
            },
        )
    }
}
