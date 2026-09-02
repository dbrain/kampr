package dev.kampr.conversation

import androidx.compose.runtime.staticCompositionLocalOf

// A media type reduced to the part a decoder is asked about. A `Content-Type` may carry parameters
// and a record may carry case, and neither belongs in a lookup.
fun audioType(mime: String?): String? = mime
    ?.substringBefore(';')
    ?.trim()
    ?.lowercase()
    ?.takeIf { it.startsWith("audio/") }

// A sound the device has taken, and which can be started and stopped.
//
// `playing` is polled rather than pushed. Completion is a listener on one target, a promise on
// another and a line event on the third, and the only thing the card does with the answer is put
// the button back — so a question asked four times a second while a sound is running costs less
// than three shapes of callback.
interface Voice {
    fun play()
    fun pause()
    fun release()
    val playing: Boolean
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

    // Null where the device took the bytes and could not make a sound of them. [canPlay] cannot
    // rule that out: it answers for a type, and what is behind a type is whatever was on the disk.
    fun open(bytes: ByteArray, mime: String?): Voice?
}

expect val deviceVoices: Voices

val LocalVoices = staticCompositionLocalOf { deviceVoices }
