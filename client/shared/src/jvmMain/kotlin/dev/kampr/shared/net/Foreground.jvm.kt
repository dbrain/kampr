package dev.kampr.shared.net

// A desktop window is never frozen the way a phone's process is, and the socket outlives a
// minimised window, so there is nothing here to watch for.
actual fun watchForeground(onForeground: () -> Unit): ForegroundWatch = ForegroundWatch {}
