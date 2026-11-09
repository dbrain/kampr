package dev.kampr.conversation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics

// A decoder this test drives, in place of the device's own. The desktop's is deliberately not used
// here: opening a JDK audio line starts AWT inside the test JVM and deadlocks the Compose harness,
// which is the whole reason [LocalVoices] exists — see the comment on `Audio.jvm.kt`.
internal class Decoder(
    private val types: Set<String> = setOf("audio/wav"),
    private val deaf: Boolean = false,
    private val lasts: Long? = null,
) : Voices {
    var latest: Sounding? = null

    override fun canPlay(mime: String?): Boolean = audioType(mime) in types

    override fun stream(url: String, mime: String?): Voice = Sounding(url, deaf, lasts).also { latest = it }
}

internal class Sounding(val url: String, failed: Boolean, override val durationMs: Long?) : Voice {
    override var playing = false
        private set
    override var positionMs = 0L
        private set
    override var failed = failed
        private set

    fun ended() {
        playing = false
    }

    fun dropped() {
        failed = true
        playing = false
    }

    override fun play() {
        if (!failed) playing = true
    }

    override fun pause() {
        playing = false
    }

    override fun seek(ms: Long) {
        positionMs = ms
    }

    override fun release() {
        playing = false
    }
}

// A video player this test can see, in place of a `<video>` element or an Android `VideoView`.
internal class Projector(private val types: Set<String> = setOf("video/mp4")) : Films {
    var shown: String? = null
    private var failing: (() -> Unit)? = null

    fun dropped() = requireNotNull(failing) { "no screen is showing" }()

    override fun canPlay(mime: String?): Boolean = videoType(mime) in types

    @Composable
    override fun Screen(url: String, name: String, onFailed: () -> Unit, modifier: Modifier) {
        shown = url
        failing = onFailed
        Box(modifier.fillMaxSize().semantics { contentDescription = "Playing $name" })
    }
}
