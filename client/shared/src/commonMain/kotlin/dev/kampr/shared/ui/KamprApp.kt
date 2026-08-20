package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.net.AuthApi
import dev.kampr.shared.net.DeviceRecord
import dev.kampr.shared.net.SetupStatus
import dev.kampr.shared.net.createHttpClient
import dev.kampr.shared.net.wallClockMillis
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.KamprTheme
import dev.kampr.shared.theme.ThemeId
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.util.formatLatency
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Security
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

data class DeepLink(
    val theme: String? = null,
    val screen: String? = null,
    val view: String? = null,
    val pane: String? = null,
)

@Composable
fun KamprApp(surfaces: PaneSurfaces = FallbackSurfaces, deepLink: DeepLink? = null) {
    val scope = rememberCoroutineScope()
    val state = remember { AppState(scope) }
    LaunchedEffect(state) { state.start() }
    LaunchedEffect(deepLink) {
        deepLink?.theme?.let { key -> ThemeId.entries.firstOrNull { it.key == key }?.let(state::selectTheme) }
    }

    var now by remember { mutableStateOf(wallClockMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            now = wallClockMillis()
            delay(20_000)
        }
    }

    var setup by remember { mutableStateOf<SetupStatus?>(null) }
    var devices by remember { mutableStateOf<List<DeviceRecord>>(emptyList()) }
    var deviceRefresh by remember { mutableStateOf(0) }
    val connectionStatus by state.store.status.collectAsState()
    val live = connectionStatus is ConnectionStatus.Live
    LaunchedEffect(state.endpoint.baseUrl, live, deviceRefresh) {
        if (!live) return@LaunchedEffect
        val client = createHttpClient()
        try {
            val api = AuthApi(client, state.endpoint)
            setup = api.status()
            devices = api.devices()
        } finally {
            client.close()
        }
    }

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val breakpoint = breakpointOf(maxWidth, maxHeight)
        val scale = if (breakpoint == Breakpoint.Desktop) TypeScale.Desk else TypeScale.Phone
        KamprTheme(state.theme, scale) {
            CompositionLocalProvider(LocalPaneIo provides remember(state) { AppPaneIo(state) }) {
                AppScaffold(
                    state, breakpoint, surfaces, now, setup, devices, connectionStatus, deepLink,
                    onRevoke = { id ->
                        scope.launch {
                            val client = createHttpClient()
                            try {
                                AuthApi(client, state.endpoint).revoke(id)
                            } finally {
                                client.close()
                            }
                            deviceRefresh++
                        }
                    },
                )
            }
        }
    }
}

private class AppPaneIo(private val state: AppState) : PaneIo {
    override fun send(msg: ClientMsg) = state.connection.send(msg)
    override fun prefs(paneId: String) = state.store.prefsFor(paneId)
    override val readOnly: Boolean get() = state.store.readOnly
    override fun show(view: PaneView) = state.setPaneView(view)
}

@Composable
private fun AppScaffold(
    state: AppState,
    breakpoint: Breakpoint,
    surfaces: PaneSurfaces,
    now: Double,
    setup: SetupStatus?,
    devices: List<DeviceRecord>,
    connectionStatus: ConnectionStatus,
    deepLink: DeepLink?,
    onRevoke: (String) -> Unit,
) {
    val tokens = Kampr.tokens
    val herd by state.store.herd.collectAsState()
    val hello by state.store.hello.collectAsState()
    val localRtt by state.store.localRttMs.collectAsState()
    val security = hello?.security ?: Security()
    val readOnly = hello?.role == "readonly"
    val failure by state.store.failure.collectAsState()
    val blocked = herd.panes.firstOrNull { statusOf(it) == AgentStatus.Blocked }
    val blockedQuestion = blocked?.let { state.store.pane(it.id).pending?.question }

    LaunchedEffect(breakpoint, herd.known, deepLink) {
        val target = when (deepLink?.screen) {
            "setup" -> Screen.Setup
            "devices" -> Screen.Devices
            "appearance" -> Screen.Appearance
            "herd" -> Screen.Herd
            else -> null
        }
        val view = when (deepLink?.view) {
            "terminal" -> PaneView.Terminal
            "conversation" -> PaneView.Conversation
            "split" -> PaneView.Split
            else -> null
        }
        val chosen = deepLink?.pane
            ?.let { needle -> herd.panes.firstOrNull { it.id.contains(needle) || (it.workspace ?: "") == needle } }
            ?: blocked
            ?: herd.panes.firstOrNull()
        when {
            target != null -> state.go(target)
            (deepLink?.view != null || deepLink?.pane != null) && chosen != null ->
                state.openPane(chosen.id, view)
            breakpoint == Breakpoint.Desktop && state.screen is Screen.Herd && chosen != null ->
                state.openPane(chosen.id, PaneView.Split)
        }
    }

    fun answer(paneId: String, key: String) {
        state.connection.send(ClientMsg.Answer(paneId, key))
    }

    Box(Modifier.fillMaxSize().background(tokens.color.bg)) {
        when (breakpoint) {
            Breakpoint.Desktop -> Column(Modifier.fillMaxSize()) {
                Row(Modifier.weight(1f)) {
                    HerdSidebar(
                        herd = herd,
                        now = now,
                        localRtt = localRtt,
                        blocked = blocked,
                        blockedQuestion = blockedQuestion,
                        activePaneId = (state.screen as? Screen.Pane)?.paneId,
                        deviceName = devices.firstOrNull { it.current }?.name ?: "this device",
                        deviceDetail = hello?.let { "${it.role} access · ${it.build}" } ?: "not connected",
                        onOpenPane = { state.openPane(it, PaneView.Split) },
                        onSettings = { state.go(Screen.Appearance) },
                    )
                    Box(Modifier.weight(1f).fillMaxSize()) {
                        when (val screen = state.screen) {
                            is Screen.Pane -> PaneScreenDesktop(
                                pane = state.store.pane(screen.paneId),
                                info = state.store.paneInfo(screen.paneId),
                                view = screen.view,
                                surfaces = surfaces,
                                readOnly = readOnly,
                                onView = state::setPaneView,
                                onAnswer = { answer(screen.paneId, it) },
                            )
                            Screen.Setup -> SetupScreen(setup, security, connectionStatus is ConnectionStatus.Live, state.endpoint, state::useEndpoint, { state.go(Screen.Herd) }, { state.go(Screen.Devices) })
                            Screen.Devices -> DevicesScreen(devices, { state.go(Screen.Herd) }, onRevoke)
                            Screen.Appearance -> AppearanceScreen(state.theme.id, 4, state::selectTheme, { state.go(Screen.Herd) })
                            Screen.Herd -> EmptyDetail(connectionStatus)
                        }
                    }
                }
                StatusStrip(state, connectionStatus, localRtt, hello?.build)
            }

            Breakpoint.Landscape -> when (val screen = state.screen) {
                is Screen.Pane -> PaneScreenMobile(
                    pane = state.store.pane(screen.paneId),
                    info = state.store.paneInfo(screen.paneId),
                    view = screen.view,
                    surfaces = surfaces,
                    landscape = true,
                    readOnly = readOnly,
                    onBack = state::back,
                    onView = state::setPaneView,
                    onAnswer = { answer(screen.paneId, it) },
                )
                Screen.Setup -> SetupScreen(setup, security, connectionStatus is ConnectionStatus.Live, state.endpoint, state::useEndpoint, { state.go(Screen.Herd) }, { state.go(Screen.Devices) })
                Screen.Devices -> DevicesScreen(devices, { state.go(Screen.Herd) }, onRevoke)
                Screen.Appearance -> AppearanceScreen(state.theme.id, 2, state::selectTheme, { state.go(Screen.Herd) })
                Screen.Herd -> HerdLandscape(herd, now, localRtt, blocked, blockedQuestion, state::openPane, null)
            }

            Breakpoint.Portrait -> Column(Modifier.fillMaxSize()) {
                Box(Modifier.weight(1f)) {
                    when (val screen = state.screen) {
                        is Screen.Pane -> PaneScreenMobile(
                            pane = state.store.pane(screen.paneId),
                            info = state.store.paneInfo(screen.paneId),
                            view = screen.view,
                            surfaces = surfaces,
                            landscape = false,
                            readOnly = readOnly,
                            onBack = state::back,
                            onView = state::setPaneView,
                            onAnswer = { answer(screen.paneId, it) },
                        )
                        Screen.Setup -> SetupScreen(setup, security, connectionStatus is ConnectionStatus.Live, state.endpoint, state::useEndpoint, { state.go(Screen.Herd) }, { state.go(Screen.Devices) })
                        Screen.Devices -> DevicesScreen(devices, { state.go(Screen.Setup) }, onRevoke)
                        Screen.Appearance -> AppearanceScreen(state.theme.id, 1, state::selectTheme, { state.go(Screen.Setup) })
                        Screen.Herd -> HerdPortrait(
                            herd, now, localRtt, blocked, blockedQuestion,
                            state::openPane,
                            if (readOnly) null else blocked?.let { pane -> { answer(pane.id, "1") } },
                        )
                    }
                }
                BottomNav(
                    when (state.screen) {
                        is Screen.Pane -> Tab.Pane
                        Screen.Herd -> Tab.Herd
                        else -> Tab.Nodes
                    },
                    state::selectTab,
                )
            }
        }

        failure?.let { ErrorStrip(it.message, it.code, state.store::dismissFailure) }
    }
}

// error.code is an open string: an unrecognised code still shows its message.
@Composable
private fun BoxScope.ErrorStrip(message: String, code: String, onDismiss: () -> Unit) {
    val tokens = Kampr.tokens
    Row(
        Modifier
            .align(Alignment.TopCenter)
            .padding(12.dp)
            .background(tokens.color.blockedBg, RoundedCornerShape(tokens.radii.md))
            .edge(BorderSpec(1.dp, tokens.color.blocked), RoundedCornerShape(tokens.radii.md))
            .clickable(onClick = onDismiss)
            .padding(horizontal = 14.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(9.dp),
    ) {
        IconGlyph(KamprIcons.warning, 14.dp, tokens.color.blocked)
        KText(message.ifBlank { code }, tokens.type.caption, tokens.color.text)
        KText(code, tokens.type.meta, tokens.color.mute)
    }
}

@Composable
private fun EmptyDetail(connectionStatus: ConnectionStatus) {
    val tokens = Kampr.tokens
    Box(Modifier.fillMaxSize().background(tokens.color.surface2), contentAlignment = Alignment.Center) {
        KText(
            when (connectionStatus) {
                is ConnectionStatus.Live -> "Pick a pane"
                is ConnectionStatus.Offline -> "Reconnecting"
                ConnectionStatus.Connecting -> "Connecting"
                ConnectionStatus.Idle -> "Not connected"
            },
            tokens.type.caption,
            tokens.color.mute,
        )
    }
}

@Composable
private fun StatusStrip(
    state: AppState,
    connectionStatus: ConnectionStatus,
    localRtt: Double?,
    build: String?,
) {
    val tokens = Kampr.tokens
    val herd by state.store.herd.collectAsState()
    val hub = herd.nodes.firstOrNull { it.kind == "local" }
    Row(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeTop()
            .padding(horizontal = 18.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(18.dp),
    ) {
        val live = connectionStatus is ConnectionStatus.Live
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(7.dp),
        ) {
            Dot(if (live) tokens.color.done else tokens.color.working, 6.dp)
            KText("hub · ${hub?.name ?: "—"}", tokens.type.meta, if (live) tokens.color.done else tokens.color.working)
        }
        for (node in herd.nodes.filter { it.kind != "local" }) {
            KText("${node.name} ${formatLatency(node.rttMs)}", tokens.type.meta, tokens.color.mute)
        }
        Box(Modifier.weight(1f))
        KText(
            when (connectionStatus) {
                is ConnectionStatus.Offline -> "reconnecting in ${connectionStatus.retryInMs / 1000}s — showing cached grid"
                else -> "no lease held — desktop shape untouched"
            },
            tokens.type.meta,
            tokens.color.mute,
        )
        KText("local ${formatLatency(localRtt)}", tokens.type.meta, tokens.color.mute)
        KText("kampr ${build ?: "0.1.0"}", tokens.type.meta, tokens.color.mute)
    }
}
