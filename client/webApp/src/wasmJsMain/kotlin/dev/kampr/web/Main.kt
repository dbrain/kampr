package dev.kampr.web

import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.window.ComposeViewport
import dev.kampr.shared.ui.DeepLink
import dev.kampr.shared.ui.KamprApp
import dev.kampr.conversation.ConversationSurfaces
import dev.kampr.mosaic.MosaicSurfaces
import dev.kampr.terminal.TerminalSurfaces
import dev.kampr.terminal.bench.TerminalBenchApp
import kotlinx.browser.document
import kotlinx.browser.window

private fun param(source: String, name: String): String? {
    if (source.isEmpty()) return null
    return source.split('&')
        .mapNotNull { it.split('=', limit = 2).takeIf { parts -> parts.size == 2 } }
        .firstOrNull { it[0] == name }
        ?.get(1)
}

private fun query(name: String): String? = param(window.location.search.removePrefix("?"), name)

// A pairing code rides in the fragment, never the query: a fragment is not sent to the node, so
// it cannot land in its access log or in the reverse proxy's.
private fun fragment(name: String): String? = param(window.location.hash.removePrefix("#"), name)

// ConversationSurfaces wraps: it renders the transcript and delegates the terminal and the
// key row to its base, so both halves of the pane are live.
private val surfaces = ConversationSurfaces(TerminalSurfaces())
private val mosaic = MosaicSurfaces()

@OptIn(ExperimentalComposeUiApi::class)
fun main() {
    val bench = query("bench") != null
    val deepLink = DeepLink(
        query("theme"), query("mode"), query("screen"), query("view"), query("pane"),
        fragment("pair"),
    )
    ComposeViewport(document.body!!) {
        if (bench) TerminalBenchApp() else KamprApp(surfaces, deepLink, mosaic)
    }
}
