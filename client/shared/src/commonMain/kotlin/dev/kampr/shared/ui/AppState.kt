package dev.kampr.shared.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.AuthApi
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.net.createHttpClient
import dev.kampr.shared.net.defaultEndpoint
import dev.kampr.shared.platform.Prefs
import dev.kampr.shared.platform.createPrefs
import dev.kampr.shared.push.PushPlatform
import dev.kampr.shared.push.createPushPlatform
import dev.kampr.shared.theme.ThemeId
import dev.kampr.shared.theme.ThemeMode
import dev.kampr.shared.theme.ThemeSpec
import dev.kampr.shared.theme.modeOf
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.wire.ClientMsg
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

enum class PaneView(val key: String) {
    Terminal("terminal"),
    Conversation("conversation"),
    Split("split"),
}

fun viewOf(key: String): PaneView? = PaneView.entries.firstOrNull { it.key == key }

enum class Tab { Herd, Pane, Nodes }

sealed interface Screen {
    data object Herd : Screen
    data class Pane(val paneId: String, val view: PaneView) : Screen
    data object Setup : Screen
    data object Devices : Screen
    data object Appearance : Screen
    data object Notifications : Screen
}

private const val KEY_THEME = "theme"
private const val KEY_MODE = "mode"
private const val KEY_ENDPOINT = "endpoint"
private const val KEY_TOKEN = "token"

class AppState(
    private val scope: CoroutineScope,
    val store: KamprStore = KamprStore(),
    private val prefs: Prefs = createPrefs(),
) {
    val connection = KamprConnection(scope, store)

    // The service worker outlives every page and cannot read where the token is kept, so it is
    // handed over on the way past. Without it a push arrives, warms nothing, and the tap that
    // follows loads from cold.
    val push: PushPlatform = createPushPlatform()

    var theme: ThemeSpec by mutableStateOf(themeOf(prefs.get(KEY_THEME)))
        private set

    var themeMode: ThemeMode by mutableStateOf(modeOf(prefs.get(KEY_MODE)))
        private set

    var screen: Screen by mutableStateOf(Screen.Herd)
        private set

    var lastPaneId: String? by mutableStateOf(null)
        private set

    val endpoint: Endpoint
        get() {
            val saved = prefs.get(KEY_ENDPOINT) ?: return defaultEndpoint()
            return Endpoint(saved, prefs.get(KEY_TOKEN))
        }

    fun start() {
        val target = endpoint
        push.prepare(target.token)
        connection.connect(target)
    }

    fun useEndpoint(target: Endpoint) {
        scope.launch {
            val token = target.token?.let { code -> exchange(target, code) } ?: target.token
            val resolved = target.copy(token = token)
            prefs.set(KEY_ENDPOINT, resolved.baseUrl)
            prefs.set(KEY_TOKEN, resolved.token)
            push.prepare(resolved.token)
            connection.connect(resolved)
        }
    }

    private suspend fun exchange(target: Endpoint, code: String): String {
        val client = createHttpClient()
        return try {
            AuthApi(client, target).pair(code) ?: code
        } finally {
            client.close()
        }
    }

    fun selectTheme(id: ThemeId) {
        theme = themeOf(id.key)
        prefs.set(KEY_THEME, id.key)
    }

    fun selectMode(mode: ThemeMode) {
        themeMode = mode
        prefs.set(KEY_MODE, mode.key)
    }

    fun go(target: Screen) {
        if (target is Screen.Pane) {
            lastPaneId = target.paneId
            connection.watch(target.paneId)
        }
        screen = target
    }

    fun openPane(paneId: String, prefer: PaneView? = null) {
        val info = store.paneInfo(paneId)
        val noRing = (info?.scrollbackRows ?: 0) == 0
        val remembered = store.prefsFor(paneId).view?.let(::viewOf)
        val view = prefer
            ?: remembered
            ?: if (info?.hasConversation == true && noRing) PaneView.Conversation else PaneView.Terminal
        go(Screen.Pane(paneId, view))
    }

    fun setPaneView(view: PaneView) {
        val current = screen
        if (current !is Screen.Pane) return
        screen = current.copy(view = view)
        connection.send(ClientMsg.SetPrefs(current.paneId, mapOf("view" to view.key)))
    }

    fun selectTab(tab: Tab) {
        when (tab) {
            Tab.Herd -> go(Screen.Herd)
            Tab.Pane -> lastPaneId?.let { openPane(it) } ?: go(Screen.Herd)
            Tab.Nodes -> go(Screen.Setup)
        }
    }

    fun back() {
        go(Screen.Herd)
    }
}
