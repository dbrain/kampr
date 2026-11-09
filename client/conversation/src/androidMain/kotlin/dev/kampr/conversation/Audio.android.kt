package dev.kampr.conversation

import android.media.AudioAttributes
import android.media.MediaPlayer

// The types Android's own media framework is documented to decode on every device from the
// minimum SDK up. Anything else is the download.
private val PLAYABLE = setOf(
    "audio/wav",
    "audio/x-wav",
    "audio/vnd.wave",
    "audio/wave",
    "audio/mpeg",
    "audio/mp4",
    "audio/aac",
    "audio/ogg",
    "audio/flac",
    "audio/x-flac",
)

actual val deviceVoices: Voices = object : Voices {
    override fun canPlay(mime: String?): Boolean = audioType(mime) in PLAYABLE

    override fun stream(url: String, mime: String?): Voice = AndroidVoice(url)
}

// `prepareAsync`, never `prepare`: preparing a URL reads the start of the stream, and that is a
// network round trip on whichever thread asked. Until it is done a play or a seek is remembered and
// applied on arrival, so the first press is never lost to a phone link that took a second.
private class AndroidVoice(url: String) : Voice {
    private val player = MediaPlayer()
    private var prepared = false
    private var released = false
    private var wanted = false
    private var pendingSeek: Long? = null

    override var failed = false
        private set

    init {
        runCatching {
            player.setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                    .build(),
            )
            player.setOnPreparedListener {
                prepared = true
                pendingSeek?.let { at -> player.seekTo(at.toInt()) }
                pendingSeek = null
                if (wanted) player.start()
            }
            player.setOnErrorListener { _, _, _ ->
                failed = true
                wanted = false
                true
            }
            player.setOnCompletionListener { wanted = false }
            player.setDataSource(url)
            player.prepareAsync()
        }.onFailure { failed = true }
    }

    override fun play() {
        if (released || failed) return
        wanted = true
        if (!prepared) return
        runCatching {
            if (player.currentPosition >= player.duration) player.seekTo(0)
            player.start()
        }
    }

    override fun pause() {
        wanted = false
        if (prepared && !released) runCatching { player.pause() }
    }

    override fun seek(ms: Long) {
        if (released) return
        if (!prepared) {
            pendingSeek = ms
            return
        }
        runCatching { player.seekTo(ms.toInt()) }
    }

    override fun release() {
        if (released) return
        released = true
        runCatching { player.release() }
    }

    // A press still waiting on the stream is reported as playing, or the poll would put the play
    // button back while the first second of a slow link is still on its way.
    override val playing: Boolean
        get() = when {
            released || failed -> false
            !prepared -> wanted
            else -> runCatching { player.isPlaying }.getOrDefault(false)
        }

    override val positionMs: Long
        get() = if (prepared && !released) runCatching { player.currentPosition.toLong() }.getOrDefault(0L) else 0L

    override val durationMs: Long?
        get() = if (prepared && !released) runCatching { player.duration.toLong() }.getOrNull()?.takeIf { it > 0 } else null
}
