package dev.kampr.shared.net

fun interface ForegroundWatch {
    fun stop()
}

expect fun watchForeground(onForeground: () -> Unit): ForegroundWatch
