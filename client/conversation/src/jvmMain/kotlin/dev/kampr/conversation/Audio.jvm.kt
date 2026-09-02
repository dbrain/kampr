package dev.kampr.conversation

import java.io.ByteArrayInputStream
import javax.sound.sampled.AudioSystem
import javax.sound.sampled.Clip

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

    // A mixer is hardware. A desktop with none — a headless build machine, a container — refuses
    // the line rather than the file, and that is a device that cannot play, said at the only
    // moment it can be found out.
    override fun open(bytes: ByteArray, mime: String?): Voice? = runCatching {
        val clip = AudioSystem.getClip()
        clip.open(AudioSystem.getAudioInputStream(ByteArrayInputStream(bytes)))
        JvmVoice(clip)
    }.getOrNull()
}

private class JvmVoice(private val clip: Clip) : Voice {
    override fun play() {
        if (clip.framePosition >= clip.frameLength) clip.framePosition = 0
        clip.start()
    }

    override fun pause() {
        clip.stop()
    }

    override fun release() {
        runCatching { clip.close() }
    }

    override val playing: Boolean get() = clip.isRunning
}
