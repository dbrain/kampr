package dev.kampr.shared.platform

import kotlinx.browser.window

private class BrowserPrefs : Prefs {
    override fun get(key: String): String? = window.localStorage.getItem(key)
    override fun set(key: String, value: String?) {
        if (value == null) window.localStorage.removeItem(key) else window.localStorage.setItem(key, value)
    }
}

actual fun createPrefs(): Prefs = BrowserPrefs()
