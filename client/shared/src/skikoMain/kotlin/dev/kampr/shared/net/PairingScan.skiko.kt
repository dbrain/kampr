package dev.kampr.shared.net

import androidx.compose.runtime.Composable

// The desktop and the browser build reach a node by having its address typed or its link opened,
// and the phone standing next to them already has a camera app that reads the same symbol.
actual val pairingScanAvailable: Boolean = false

@Composable
actual fun PairingScanSurface(onScanned: (String) -> Unit, onClose: () -> Unit) {
    onClose()
}
