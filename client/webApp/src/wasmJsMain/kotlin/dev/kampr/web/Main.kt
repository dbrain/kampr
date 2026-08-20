package dev.kampr.web

import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.window.ComposeViewport
import dev.kampr.shared.ui.DeepLink
import dev.kampr.shared.ui.KamprApp
import kotlinx.browser.document
import kotlinx.browser.window

private fun query(name: String): String? {
    val search = window.location.search.removePrefix("?")
    if (search.isEmpty()) return null
    return search.split('&')
        .mapNotNull { it.split('=', limit = 2).takeIf { parts -> parts.size == 2 } }
        .firstOrNull { it[0] == name }
        ?.get(1)
}

@OptIn(ExperimentalComposeUiApi::class)
fun main() {
    val deepLink = DeepLink(query("theme"), query("screen"), query("view"), query("pane"))
    ComposeViewport(document.body!!) {
        KamprApp(deepLink = deepLink)
    }
}
