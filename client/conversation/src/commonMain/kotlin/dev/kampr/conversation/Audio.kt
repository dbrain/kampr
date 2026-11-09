package dev.kampr.conversation

import androidx.compose.runtime.staticCompositionLocalOf

// A media type reduced to the part a decoder is asked about. A `Content-Type` may carry parameters
// and a record may carry case, and neither belongs in a lookup.
fun audioType(mime: String?): String? = mime
    ?.substringBefore(';')
    ?.trim()
    ?.lowercase()
    ?.takeIf { it.startsWith("audio/") }

// A recording the device is playing from the node, which can be started, stopped and moved through.
//
// Everything about it is polled rather than pushed. Completion, failure and position are a
// listener on one target, a promise or event on another and a line event on the third, and all the
// player does with them is redraw a button and a clock — so a question asked a few times a second
// costs less than three shapes of callback.
interface Voice {
    fun play()
    fun pause()
    fun seek(ms: Long)
    fun release()
    val playing: Boolean

    // The decoder gave up on what the node sent. Asked as well as [playing] because a device can
    // claim a type and still be handed bytes behind it that it cannot read.
    val failed: Boolean
    val positionMs: Long

    // Null until the stream has said how long it is, and for a stream that never will.
    val durationMs: Long?
}

// What this device can do with a recording.
//
// An interface rather than two top-level functions because the desktop's decoder cannot be run in
// this project's own tests — see the comment on the JVM implementation — so the surface that draws
// the player has to be drivable from a decoder a test supplies.
interface Voices {
    // Whether this device can turn that media type into sound at all, asked before anything is
    // offered. The three targets have three decoders and none of them is a promise the others
    // make: a browser answers for itself, Android's `MediaPlayer` has a documented list, and the
    // JDK reads three container formats and no compressed one. An affordance that cannot work must
    // be absent rather than present-and-failing, and this is what decides.
    fun canPlay(mime: String?): Boolean

    // Played from a URL that needs no header, because no media element can send one: the node's
    // stream ticket. It returns at once and loads on its own thread, so composing a player never
    // waits on the network.
    fun stream(url: String, mime: String?): Voice
}

expect val deviceVoices: Voices

val LocalVoices = staticCompositionLocalOf { deviceVoices }
