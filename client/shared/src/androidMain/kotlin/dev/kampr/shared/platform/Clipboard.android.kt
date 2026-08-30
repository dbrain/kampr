package dev.kampr.shared.platform

import android.content.ClipboardManager
import android.content.Context

// Reading the primary clip is what raises Android's own "pasted from your clipboard" notice, so it
// happens on the press and never to decide whether to offer one — a Paste control that had to ask
// the clipboard whether to draw itself would accuse the app of reading it every time a pill opened.
//
// `coerceToText` rather than `text`: a URI or an intent copied from another app has a readable
// form, and it is the form that app put on the clipboard for pasting.
actual suspend fun clipboardText(): String? {
    val context = KamprAndroid.context ?: return null
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return null
    val clip = clipboard.primaryClip ?: return null
    return (0 until clip.itemCount)
        .asSequence()
        .mapNotNull { clip.getItemAt(it).coerceToText(context)?.toString() }
        .firstOrNull { it.isNotEmpty() }
}
