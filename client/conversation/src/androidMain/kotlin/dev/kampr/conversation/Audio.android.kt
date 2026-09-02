package dev.kampr.conversation

import android.media.MediaDataSource
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

    override fun open(bytes: ByteArray, mime: String?): Voice? = runCatching {
        val player = MediaPlayer()
        player.setDataSource(Held(bytes))
        player.prepare()
        AndroidVoice(player)
    }.getOrNull()
}

// Fed from memory rather than from a file. The bytes are already held for the download beside
// this, and writing them out again would be a copy in the app's cache with nothing to delete it —
// `MediaDataSource` has been the way to avoid that since API 23, and the minimum here is 26.
private class Held(private val data: ByteArray) : MediaDataSource() {
    override fun readAt(position: Long, buffer: ByteArray, offset: Int, size: Int): Int {
        if (position >= data.size) return -1
        val room = minOf(size.toLong(), data.size - position).toInt()
        data.copyInto(buffer, offset, position.toInt(), position.toInt() + room)
        return room
    }

    override fun getSize(): Long = data.size.toLong()

    override fun close() = Unit
}

private class AndroidVoice(private val player: MediaPlayer) : Voice {
    private var released = false

    override fun play() {
        if (released) return
        runCatching {
            if (player.currentPosition >= player.duration) player.seekTo(0)
            player.start()
        }
    }

    override fun pause() {
        if (!released) runCatching { player.pause() }
    }

    override fun release() {
        if (released) return
        released = true
        runCatching { player.release() }
    }

    override val playing: Boolean
        get() = !released && runCatching { player.isPlaying }.getOrDefault(false)
}
