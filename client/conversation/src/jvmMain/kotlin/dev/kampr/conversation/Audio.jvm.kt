package dev.kampr.conversation

import java.io.BufferedInputStream
import java.net.URI
import javax.sound.sampled.AudioSystem
import javax.sound.sampled.Clip
import kotlin.concurrent.thread

// What `javax.sound.sampled` reads without a service provider on the classpath, and it is three
// container formats and no compressed one: there is no MP3, AAC, FLAC or Vorbis decoder in the
// JDK. A desktop reader is offered the download for those rather than a button that would open
// nothing.
private val PLAYABLE = setOf(
    "audio/wav",
    "audio/x-wav",
    "audio/vnd.wave",
    "audio/wave",
    "audio/aiff",
    "audio/x-aiff",
    "audio/basic",
)

// **Nothing in this project's own tests may open one of these lines.** Opening a JDK audio line
// starts `com.sun.media.sound.EventDispatcher` and, with it, `AWT-EventQueue-0` — and a second
// thread driving Compose is all it takes to deadlock the skiko test harness: the Test worker holds
// `FlushCoroutineDispatcher`'s lock and waits for the snapshot lock while the AWT thread holds the
// snapshot lock and waits for the dispatcher's. The JVM reports it as a Java-level deadlock, and
// it reproduced twice under a parallel `gradlew` run and never once with this decoder stubbed out.
// [LocalVoices] is how a test supplies its own instead.
actual val deviceVoices: Voices = object : Voices {
    override fun canPlay(mime: String?): Boolean = audioType(mime) in PLAYABLE

    override fun stream(url: String, mime: String?): Voice = JvmVoice(url)
}

// A `Clip` holds the whole recording, so the desktop reads all of it before the first sound; it is
// read on a thread of its own so that composing the player never waits on it. A mixer is hardware,
// and a desktop with none — a headless build machine, a container — refuses the line rather than
// the file, which is a device that cannot play, said at the only moment it can be found out.
private class JvmVoice(url: String) : Voice {
    @Volatile private var clip: Clip? = null

    @Volatile private var wanted = false

    @Volatile private var released = false

    @Volatile override var failed = false
        private set

    init {
        thread(isDaemon = true, name = "kampr-voice") {
            runCatching {
                val opened = AudioSystem.getClip()
                opened.open(AudioSystem.getAudioInputStream(BufferedInputStream(URI(url).toURL().openStream())))
                opened
            }.onSuccess { opened ->
                if (released) {
                    opened.close()
                } else {
                    clip = opened
                    if (wanted) opened.start()
                }
            }.onFailure { failed = true }
        }
    }

    override fun play() {
        wanted = true
        val c = clip ?: return
        if (c.framePosition >= c.frameLength) c.framePosition = 0
        c.start()
    }

    override fun pause() {
        wanted = false
        clip?.stop()
    }

    override fun seek(ms: Long) {
        clip?.microsecondPosition = ms * 1000
    }

    override fun release() {
        released = true
        runCatching { clip?.close() }
    }

    override val playing: Boolean get() = clip?.isRunning ?: (wanted && !failed)

    override val positionMs: Long get() = (clip?.microsecondPosition ?: 0L) / 1000

    override val durationMs: Long? get() = clip?.microsecondLength?.takeIf { it > 0 }?.div(1000)
}
