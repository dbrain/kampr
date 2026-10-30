package dev.kampr.shared.net

fun interface ForegroundWatch {
    fun stop()
}

// `true` each time somebody comes back to the app, `false` when nobody can see it any more.
expect fun watchForeground(onChange: (shown: Boolean) -> Unit): ForegroundWatch
