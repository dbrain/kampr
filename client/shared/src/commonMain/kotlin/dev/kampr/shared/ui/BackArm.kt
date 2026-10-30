package dev.kampr.shared.ui

internal interface BrowserHistory {
    fun push()
    fun back()
}

// A browser's back button cannot be claimed, only fed: it traverses history and the page hears a
// popstate after the fact. So while the app has somewhere to go back to, exactly one entry is kept
// above the one it was opened on, and arriving back on that lower entry is the gesture. At the root
// nothing is kept, so the same button leaves the site as it would anywhere else.
internal class BackArm(
    private val history: BrowserHistory,
    onArmedEntry: Boolean,
    private val wanted: () -> Boolean,
    private val back: () -> Unit,
) {
    private var armed = onArmedEntry
    private var stepping = false

    fun sync() {
        if (stepping) return
        val want = wanted()
        when {
            want && !armed -> {
                history.push()
                armed = true
            }

            !want && armed -> {
                stepping = true
                history.back()
            }
        }
    }

    fun popped(onArmedEntry: Boolean) {
        val gesture = !stepping && armed && !onArmedEntry
        stepping = false
        armed = onArmedEntry
        if (gesture) back()
        sync()
    }
}
