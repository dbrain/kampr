package dev.kampr.conversation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import dev.kampr.conversation.md.pathOfLink
import dev.kampr.shared.model.resolvePath
import dev.kampr.shared.ui.LocalOpenPlace
import dev.kampr.shared.ui.filePlace

@Composable
fun PathLinks(paneId: String, cwd: String?, content: @Composable () -> Unit) {
    val browser = LocalUriHandler.current
    val open = LocalOpenPlace.current
    val handler = remember(browser, open, paneId, cwd) {
        object : UriHandler {
            override fun openUri(uri: String) {
                val path = pathOfLink(uri) ?: return browser.openUri(uri)
                open(filePlace(paneId, resolvePath(path, cwd)))
            }
        }
    }
    CompositionLocalProvider(LocalUriHandler provides handler, content = content)
}
