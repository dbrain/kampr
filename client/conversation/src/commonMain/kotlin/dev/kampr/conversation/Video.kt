package dev.kampr.conversation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier

fun videoType(mime: String?): String? = mime
    ?.substringBefore(';')
    ?.trim()
    ?.lowercase()
    ?.takeIf { it.startsWith("video/") }

// What this device can do with a film. The player is the platform's own — a `<video>` element laid
// over the canvas in a browser, a `VideoView` on Android — with its own controls, because seeking,
// fullscreen and volume are what those already do well and what a canvas drawing of them would do
// worse. An interface for the same reason [Voices] is one: the surface that shows the screen has to
// be drivable from a player a test supplies.
interface Films {
    // Asked of the device's own decoder before anything is offered, as for a recording.
    fun canPlay(mime: String?): Boolean

    // Played from the node's stream ticket, a URL that needs no header. `onFailed` is the player
    // giving up on it, which a ticket the node no longer holds makes it do.
    @Composable
    fun Screen(url: String, name: String, onFailed: () -> Unit, modifier: Modifier)
}

expect val deviceFilms: Films

val LocalFilms = staticCompositionLocalOf { deviceFilms }
