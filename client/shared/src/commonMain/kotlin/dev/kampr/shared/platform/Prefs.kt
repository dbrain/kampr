package dev.kampr.shared.platform

interface Prefs {
    fun get(key: String): String?
    fun set(key: String, value: String?)
}

expect fun createPrefs(): Prefs

class MemoryPrefs : Prefs {
    private val map = HashMap<String, String>()
    override fun get(key: String): String? = map[key]
    override fun set(key: String, value: String?) {
        if (value == null) map.remove(key) else map[key] = value
    }
}
