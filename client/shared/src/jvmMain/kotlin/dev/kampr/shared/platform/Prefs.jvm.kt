package dev.kampr.shared.platform

import java.util.prefs.Preferences

private class JvmPrefs : Prefs {
    private val node: Preferences = Preferences.userRoot().node("dev/kampr/client")
    override fun get(key: String): String? = node.get(key, null)
    override fun set(key: String, value: String?) {
        if (value == null) node.remove(key) else node.put(key, value)
    }
}

actual fun createPrefs(): Prefs = JvmPrefs()
