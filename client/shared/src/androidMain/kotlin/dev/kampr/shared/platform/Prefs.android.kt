package dev.kampr.shared.platform

import android.content.Context
import android.content.SharedPreferences

object KamprAndroid {
    internal var context: Context? = null

    fun attach(context: Context) {
        this.context = context.applicationContext
    }
}

private class AndroidPrefs(private val store: SharedPreferences) : Prefs {
    override fun get(key: String): String? = store.getString(key, null)
    override fun set(key: String, value: String?) {
        store.edit().apply { if (value == null) remove(key) else putString(key, value) }.apply()
    }
}

actual fun createPrefs(): Prefs {
    val context = KamprAndroid.context ?: return MemoryPrefs()
    return AndroidPrefs(context.getSharedPreferences("kampr", Context.MODE_PRIVATE))
}
