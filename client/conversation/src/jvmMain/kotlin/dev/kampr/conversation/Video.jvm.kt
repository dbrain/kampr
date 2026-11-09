package dev.kampr.conversation

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

// The JDK has no video decoder at all, so the desktop offers every film as a download.
actual val deviceFilms: Films = object : Films {
    override fun canPlay(mime: String?): Boolean = false

    @Composable
    override fun Screen(url: String, name: String, onFailed: () -> Unit, modifier: Modifier) = Unit
}
