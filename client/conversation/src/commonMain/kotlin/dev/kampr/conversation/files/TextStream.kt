package dev.kampr.conversation.files

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.net.AttachmentSlice
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

const val SLICE_BYTES = 2L * 1024 * 1024

// What Copy will fetch the rest of a file for. Past it the clipboard is not where the file is
// going, and Copy takes the lines already here and says so.
const val COPY_MOST = 64L * 1024 * 1024

// A text file read a slice at a time as the reader scrolls towards its end. Each slice starts where
// the bytes so far end, not where the last request asked to end: a node may answer short.
class TextStream(first: AttachmentSlice.Ok, private val next: suspend (start: Long, endInclusive: Long) -> AttachmentSlice) {
    val lines = TextLines()
    val total: Long? = first.total
    private val gate = Mutex()

    var version by mutableIntStateOf(0)
        private set
    var failed by mutableStateOf<String?>(null)
    var loading by mutableStateOf(false)
        private set

    val complete: Boolean get() = lines.ended

    init {
        take(first.bytes)
    }

    private fun take(bytes: ByteArray) {
        if (bytes.isEmpty()) lines.end() else lines.append(bytes)
        if (total != null && lines.bytes >= total) lines.end()
        version++
    }

    suspend fun more() = gate.withLock {
        if (complete || failed != null) return@withLock
        loading = true
        when (val got = next(lines.bytes, lines.bytes + SLICE_BYTES - 1)) {
            is AttachmentSlice.Ok -> take(got.bytes)
            is AttachmentSlice.Failed -> failed = got.reason
        }
        loading = false
    }

    suspend fun all(): Boolean {
        while (!complete && failed == null) more()
        return complete
    }

    companion object {
        fun whole(bytes: ByteArray, mime: String?) = TextStream(AttachmentSlice.Ok(bytes, mime, bytes.size.toLong())) { _, _ ->
            AttachmentSlice.Failed("Nothing more to read.")
        }
    }
}
