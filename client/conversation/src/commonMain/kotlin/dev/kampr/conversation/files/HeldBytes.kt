package dev.kampr.conversation.files

import androidx.compose.runtime.staticCompositionLocalOf
import dev.kampr.shared.net.AttachmentBytes

private const val MOST_HELD = 6
private const val MOST_BYTES_HELD = 24L * 1024 * 1024

// What the transcript already fetched, so opening a picture it is showing is not a second
// authorised round trip for the same 730 KB on a phone link. Records only: a record id names bytes
// that never change, and a path is read again every time it is opened.
class HeldBytes(private val most: Int = MOST_HELD, private val budget: Long = MOST_BYTES_HELD) {
    private val held = LinkedHashMap<String, AttachmentBytes.Ok>()

    fun get(anchor: String, id: String): AttachmentBytes.Ok? = held[key(anchor, id)]

    fun put(anchor: String, id: String, got: AttachmentBytes.Ok) {
        val key = key(anchor, id)
        held.remove(key)
        held[key] = got
        while (held.size > 1 && (held.size > most || held.values.sumOf { it.bytes.size.toLong() } > budget)) {
            held.remove(held.keys.first())
        }
    }

    private fun key(anchor: String, id: String) = "$anchor\u001F$id"
}

val LocalHeldBytes = staticCompositionLocalOf<HeldBytes?> { null }
