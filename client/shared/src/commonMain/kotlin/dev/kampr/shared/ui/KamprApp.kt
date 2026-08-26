package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.absolutePadding
import androidx.compose.foundation.layout.padding
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
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.net.AuthApi
import dev.kampr.shared.net.DeviceRecord
import dev.kampr.shared.net.SetupStatus
import dev.kampr.shared.net.createHttpClient
import dev.kampr.shared.net.wallClockMillis
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.KamprTheme
import dev.kampr.shared.theme.ThemeId
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.groundOf
import dev.kampr.shared.theme.modeOf
import dev.kampr.shared.util.formatLatency
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Security
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

data class DeepLink(
    val theme: String? = null,
    val mode: String? = null,
    val screen: String? = null,
    val view: String? = null,
    val pane: String? = null,
    // Carried in the fragment of a scanned pairing link, so it never reaches the node's logs or
    // any proxy in front of it. It arms the field; redeeming it is still a deliberate tap.
    val pair: String? = null,
)

@Composable
fun KamprApp(
    surfaces: PaneSurfaces = FallbackSurfaces,
    deepLink: DeepLink? = null,
    mosaic: MosaicHost = NoMosaic,
) {
    val scope = rememberCoroutineScope()
    val state = remember { AppState(scope) }
    LaunchedEffect(state) { state.start() }
    LaunchedEffect(deepLink) {
        deepLink?.theme?.let { key -> ThemeId.entries.firstOrNull { it.key == key }?.let(state::selectTheme) }
        deepLink?.mode?.let { key -> state.selectMode(modeOf(key)) }
        if (deepLink?.pair != null) state.go(Screen.Setup)
    }

    var deviceClock by remember { mutableStateOf(wallClockMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            deviceClock = wallClockMillis()
            delay(20_000)
        }
    }
    val now = deviceClock + state.clockOffsetMs

    var setup by remember { mutableStateOf<SetupStatus?>(null) }
    var devices by remember { mutableStateOf<List<DeviceRecord>>(emptyList()) }
    var pairingCode by remember { mutableStateOf<String?>(null) }
    var authFailure by remember { mutableStateOf<String?>(null) }
    var deviceRefresh by remember { mutableStateOf(0) }
    val connectionStatus by state.store.status.collectAsState()
    val live = connectionStatus is ConnectionStatus.Live
    // Not gated on a live socket: `/api/node` needs no token, and a device that has none yet is
    // exactly the one that has to be told this node takes a passkey.
    LaunchedEffect(state.endpoint?.baseUrl, live, deviceRefresh) {
        val target = state.endpoint
        if (target == null) {
            setup = null
            devices = emptyList()
            return@LaunchedEffect
        }
        val client = createHttpClient()
        try {
            val api = AuthApi(client, target)
            setup = api.status()
            devices = if (live) api.devices() else emptyList()
        } finally {
            client.close()
        }
    }

    // Every one of these ran through `runCatching{}.getOrNull()` against a route the node has
    // never had, so a refusal and a success looked identical. They do not any more.
    suspend fun <T> withApi(block: suspend (AuthApi) -> T): T? {
        val target = state.endpoint ?: return null
        val client = createHttpClient()
        return try {
            block(AuthApi(client, target))
        } finally {
            client.close()
        }
    }

    val auth = AuthSurface(
        setup = setup,
        devices = devices,
        currentDeviceId = state.deviceId,
        pairingCode = pairingCode,
        failure = authFailure,
        onPairingCode = {
            scope.launch {
                val code = withApi { it.pairingCode() }
                pairingCode = code
                if (code == null) authFailure = "This node refused to mint a pairing code."
            }
        },
        onRevoke = { id ->
            scope.launch {
                if (withApi { it.revoke(id) } != true) authFailure = "That device was not revoked."
                deviceRefresh++
            }
        },
        onRenew = { id ->
            scope.launch {
                if (withApi { it.renew(id) } != true) authFailure = "That device was not renewed."
                deviceRefresh++
            }
        },
        onDismissFailure = { authFailure = null },
    )

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val breakpoint = breakpointOf(maxWidth, maxHeight)
        val scale = if (breakpoint == Breakpoint.Desktop) TypeScale.Desk else TypeScale.Phone
        KamprTheme(state.theme, scale, groundOf(state.themeMode)) {
            CompositionLocalProvider(
                // Provided once, here, so a screen cannot forget it — and so a test can put the
                // system bars back, which is the only way this suite can see them at all.
                LocalSafeArea provides systemSafeArea(),
                LocalPaneIo provides remember(state) { AppPaneIo(state) },
                LocalManage provides remember(state) { AppManage(state) },
                LocalMosaic provides remember(state, mosaic) {
                    if (mosaic.available) ({ state.go(Screen.Mosaic) }) else null
                },
            ) {
                AppScaffold(state, breakpoint, surfaces, mosaic, now, auth, connectionStatus, deepLink)
            }
        }
    }
}

// The node's auth surface as one screen sees it. Eight parameters threaded through two layouts
// was the alternative, and every one of them is about the same three HTTP calls.
internal class AuthSurface(
    val setup: SetupStatus?,
    val devices: List<DeviceRecord>,
    val currentDeviceId: String?,
    val pairingCode: String?,
    val failure: String?,
    val onPairingCode: () -> Unit,
    val onRevoke: (String) -> Unit,
    val onRenew: (String) -> Unit,
    val onDismissFailure: () -> Unit,
)

private class AppPaneIo(private val state: AppState) : PaneIo {
    override fun send(msg: ClientMsg) = state.connection.send(msg)
    override fun prefs(paneId: String) = state.store.prefsFor(paneId)
    override fun info(paneId: String) = state.store.paneInfo(paneId)
    override val readOnly: Boolean get() = state.store.readOnly
    override fun show(view: PaneView) = state.setPaneView(view)
    override suspend fun attachment(paneId: String, id: String) = state.fetchAttachment(paneId, id)
}

@Composable
internal fun AppScaffold(
    state: AppState,
    breakpoint: Breakpoint,
    surfaces: PaneSurfaces,
    mosaic: MosaicHost,
    now: Double,
    auth: AuthSurface,
    connectionStatus: ConnectionStatus,
    deepLink: DeepLink?,
) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    val herd by state.store.herd.collectAsState()
    val hello by state.store.hello.collectAsState()
    val localRtt by state.store.localRttMs.collectAsState()
    val security = hello?.security ?: Security()
    // Off the store, not off `hello`: the role can move on a live socket and every affordance
    // below has to move with it.
    val readOnly = state.store.readOnly
    val failure by state.store.failure.collectAsState()
    // A stored token the node will not have. Not a frame — the socket never opened to carry one —
    // so it rides on the connection status, and it leads somewhere rather than only saying so.
    val refused = connectionStatus as? ConnectionStatus.Refused
    // The triage list, at last wired: `KamprStore.blocked()` had no callers, and this is what the
    // roadmap called the one Collie product idea worth stealing wholesale.
    val triage = state.store.triage()
    val blocked = triage.firstOrNull()?.pane
    // Two independent answers, both required: this origin can carry a WebAuthn RP ID, and this
    // platform has an authenticator at all. Absent rather than present-and-failing.
    val offersPasskeys = (security.passkeys || auth.setup?.passkeys == true) && state.passkeysUsable
    val passkeys = if (offersPasskeys) state::enrolPasskey else null
    val passkeySignIn = if (offersPasskeys) state::signInWithPasskey else null
    val install = if ((security.installable || auth.setup?.installable == true) && state.installable) {
        state::install
    } else {
        null
    }

    LaunchedEffect(breakpoint, herd.known, deepLink) {
        val target = when (deepLink?.screen) {
            "setup" -> Screen.Setup
            "devices" -> Screen.Devices
            "appearance" -> Screen.Appearance
            "notifications" -> Screen.Notifications
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
            // A view, not a layout: whatever this device last chose for that pane, and the
            // terminal when it has chosen nothing. Opening every pane in Split made the choice
            // unkeepable — the next open overwrote it — and is the wrong answer for most panes.
            breakpoint == Breakpoint.Desktop && state.screen is Screen.Herd && chosen != null ->
                state.openPane(chosen.id)
        }
    }

    fun answer(paneId: String, key: String) {
        state.connection.send(ClientMsg.Answer(paneId, key))
    }

    // A sheet is modal, and a screen reader has no idea unless the screen behind it stops being
    // readable. Clearing the subtree is what a focus trap buys, without turning a sheet that is
    // deliberately part of the same window into a platform dialog.
    val modal = state.sheet != null
    fun Modifier.behindSheet(): Modifier = if (modal) clearAndSetSemantics { } else this

    // Paid for once, at the root, because the keyboard covers the window rather than resizing it:
    // `enableEdgeToEdge` means `adjustResize` no longer moves anything, so every surface that ends
    // at the bottom of the window ends up underneath the keys. The conversation's reply box did.
    //
    // Here rather than per screen: this box is the only thing in the app that reaches the bottom
    // of the window, so the inset applies without a subtraction, and everything inside — the
    // bottom navigation, a sheet's fields, the pane's own key row — is measured against a window
    // that already stops at the keyboard, and against system furniture the keys have already
    // taken. `safe` is read above it, and is the window's own: the tab bar spans the edge the
    // keys are moving through, so it is sized against what the window has rather than what is
    // left of it.
    KeyboardFloor(Modifier.fillMaxSize().background(tokens.color.bg)) {
        // The mosaic is the window, not a detail pane inside it: the sidebar picks one pane,
        // and this screen is about four.
        if (state.screen is Screen.Mosaic) {
            mosaic.Mosaic(state, breakpoint, surfaces, Modifier.fillMaxSize().behindSheet())
            failure?.let { ErrorStrip(it.message, it.code, state.store::dismissFailure) }
            refused?.let { RefusedNotice(it.reason) { state.go(Screen.Setup) } }
            state.store.roleNote?.let { RoleNotice(it, state.store::dismissRoleNote) }
            ManageLayer(state, herd, breakpoint)
            return@KeyboardFloor
        }
        Box(Modifier.fillMaxSize().behindSheet()) {
            when (breakpoint) {
                Breakpoint.Desktop -> Column(Modifier.fillMaxSize().screenInset(state.screen)) {
                    Row(Modifier.weight(1f)) {
                        HerdSidebar(
                            herd = herd,
                            connection = connectionStatus,
                            now = now,
                            localRtt = localRtt,
                            triage = triage,
                            activePaneId = (state.screen as? Screen.Pane)?.paneId,
                            deviceName = auth.devices.firstOrNull { it.id == auth.currentDeviceId }?.name ?: "this device",
                            // The role off the store, not off the greeting: this caption is the
                            // only place the desktop says what this device may do.
                            deviceDetail = hello?.let { "${state.store.role} access · ${it.build}" } ?: "not connected",
                            onOpenPane = { state.openPane(it) },
                            onSettings = { state.go(Screen.Setup) },
                            onResync = { state.connection.send(ClientMsg.Resync) },
                        )
                        ScreenBody(
                            Modifier.weight(1f).fillMaxSize(),
                            bottomChrome(breakpoint, state.screen),
                            screenSelects(state.screen),
                        ) {
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
                                Screen.Setup -> SetupScreen(
                                    status = auth.setup,
                                    security = security,
                                    running = connectionStatus is ConnectionStatus.Live,
                                    endpoint = state.endpoint,
                                    nodes = herd.nodes,
                                    pairingCode = auth.pairingCode,
                                    pairingError = state.pairingError,
                                    onConnect = state::useEndpoint,
                                    onPairingCode = auth.onPairingCode,
                                    onDevices = { state.go(Screen.Devices) },
                                    onAppearance = { state.go(Screen.Appearance) },
                                    onNotifications = { state.go(Screen.Notifications) },
                                    onPasskeys = passkeys,
                                    onPasskeySignIn = passkeySignIn,
                                    onInstall = install,
                                    recentAddresses = state.recentAddresses,
                                    offeredCode = deepLink?.pair,
                                    wide = true,
                                )
                                Screen.Devices -> DevicesScreen(
                                    auth.devices, auth.currentDeviceId, now,
                                    { state.go(Screen.Setup) }, auth.onRevoke, auth.onRenew,
                                )
                                Screen.Appearance -> AppearanceScreen(state.theme.id, state.themeMode, 4, state::selectTheme, state::selectMode, onBack = { state.go(Screen.Setup) })
                                Screen.Notifications -> NotificationsScreen(state, herd.panes, onBack = { state.go(Screen.Setup) })
                                Screen.Herd, Screen.Mosaic -> EmptyDetail(connectionStatus)
                            }
                        }
                    }
                    StatusStrip(state, connectionStatus, localRtt, hello?.build)
                }

                // The nav lived in the Portrait branch alone, so a phone held sideways — a
                // first-class layout — could reach Setup, Devices, Appearance and Notifications
                // from nowhere at all. It stays off a pane, where every row of height is the
                // terminal's and `onBack` already leads out.
                Breakpoint.Landscape -> PhoneScaffold(breakpoint, state.screen, safe, state::selectTab) {
                    when (val screen = state.screen) {
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
                        Screen.Setup -> SetupScreen(
                            status = auth.setup,
                            security = security,
                            running = connectionStatus is ConnectionStatus.Live,
                            endpoint = state.endpoint,
                            nodes = herd.nodes,
                            pairingCode = auth.pairingCode,
                            pairingError = state.pairingError,
                            onConnect = state::useEndpoint,
                            onPairingCode = auth.onPairingCode,
                            onDevices = { state.go(Screen.Devices) },
                            onAppearance = { state.go(Screen.Appearance) },
                            onNotifications = { state.go(Screen.Notifications) },
                            onPasskeys = passkeys,
                            onPasskeySignIn = passkeySignIn,
                            onInstall = install,
                            recentAddresses = state.recentAddresses,
                            offeredCode = deepLink?.pair,
                            wide = false,
                        )
                        Screen.Devices -> DevicesScreen(
                            auth.devices, auth.currentDeviceId, now,
                            { state.go(Screen.Setup) }, auth.onRevoke, auth.onRenew,
                        )
                        Screen.Appearance -> AppearanceScreen(state.theme.id, state.themeMode, 2, state::selectTheme, state::selectMode, onBack = { state.go(Screen.Setup) })
                        Screen.Notifications -> NotificationsScreen(state, herd.panes, onBack = { state.go(Screen.Setup) })
                        Screen.Herd, Screen.Mosaic -> HerdLandscape(
                            herd, connectionStatus, now, localRtt, triage, state::openPane, null,
                            onResync = { state.connection.send(ClientMsg.Resync) },
                        )
                    }
                }

                Breakpoint.Portrait -> PhoneScaffold(breakpoint, state.screen, safe, state::selectTab) {
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
                        Screen.Setup -> SetupScreen(
                            status = auth.setup,
                            security = security,
                            running = connectionStatus is ConnectionStatus.Live,
                            endpoint = state.endpoint,
                            nodes = herd.nodes,
                            pairingCode = auth.pairingCode,
                            pairingError = state.pairingError,
                            onConnect = state::useEndpoint,
                            onPairingCode = auth.onPairingCode,
                            onDevices = { state.go(Screen.Devices) },
                            onAppearance = { state.go(Screen.Appearance) },
                            onNotifications = { state.go(Screen.Notifications) },
                            onPasskeys = passkeys,
                            onPasskeySignIn = passkeySignIn,
                            onInstall = install,
                            recentAddresses = state.recentAddresses,
                            offeredCode = deepLink?.pair,
                            wide = false,
                        )
                        Screen.Devices -> DevicesScreen(
                            auth.devices, auth.currentDeviceId, now,
                            { state.go(Screen.Setup) }, auth.onRevoke, auth.onRenew,
                        )
                        Screen.Appearance -> AppearanceScreen(state.theme.id, state.themeMode, 1, state::selectTheme, state::selectMode, onBack = { state.go(Screen.Setup) })
                        Screen.Notifications -> NotificationsScreen(state, herd.panes, onBack = { state.go(Screen.Setup) })
                        Screen.Herd, Screen.Mosaic -> HerdPortrait(
                            herd, connectionStatus, now, localRtt, triage,
                            state::openPane,
                            if (readOnly) null else { paneId: String -> answer(paneId, "1") },
                            onResync = { state.connection.send(ClientMsg.Resync) },
                        )
                    }
                }
            }
        }

        failure?.let { ErrorStrip(it.message, it.code, state.store::dismissFailure) }
        refused?.let { RefusedNotice(it.reason) { state.go(Screen.Setup) } }
        auth.failure?.let { ErrorStrip(it, "auth", auth.onDismissFailure) }
        state.passkeyNote?.let {
            if (it.refused) ErrorStrip(it.message, "passkey", state::dismissPasskeyNote)
            else NoteStrip(it.message, state::dismissPasskeyNote)
        }
        // A demotion or a promotion that landed on this socket. Nothing the operator did caused
        // it, so it is said here rather than discovered by pressing a control that has gone.
        state.store.roleNote?.let { RoleNotice(it, state.store::dismissRoleNote) }
        ManageLayer(state, herd, breakpoint)
    }
}

// What the system draws over, kept off every screen but the pane.
//
// The terminal paints to the edges on purpose and its own chrome decides where its controls sit,
// so padding it here would take that away; it reads `LocalSafeArea` itself. Everything else is a
// scrolling column of text that has no business under the clock — or, rotated with three-button
// navigation, under the navigation bar that has moved to the side of the screen. Chrome that
// carries the bar colour — the sidebar's title row, the desktop status strip, the bottom
// navigation — pays for its own edge instead, so its ground still runs under what the system draws.
//
// Never the bottom: whatever is below this box is the thing that owes the gesture handle.
@Composable
internal fun Modifier.screenInset(screen: Screen): Modifier {
    if (screen is Screen.Pane) return this
    val safe = LocalSafeArea.current
    return absolutePadding(left = safe.left, top = safe.top, right = safe.right)
}

@Composable
private fun EmptyDetail(connectionStatus: ConnectionStatus) {
    val tokens = Kampr.tokens
    Box(Modifier.fillMaxSize().background(tokens.color.surface2).readingOrder(1f), contentAlignment = Alignment.Center) {
        KText(connectionWord(connectionStatus) ?: "Pick a pane", tokens.type.caption, tokens.color.mute)
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
            .padding(start = 18.dp, top = 8.dp, end = 18.dp, bottom = 8.dp + LocalSafeArea.current.bottom),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(18.dp),
    ) {
        val live = connectionStatus is ConnectionStatus.Live
        Row(
            Modifier.announce(
                if (live) "Connected to hub ${hub?.name ?: "unknown"}"
                else "Not connected to hub ${hub?.name ?: "unknown"}",
            ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(7.dp),
        ) {
            Mark(
                if (live) tokens.color.done else tokens.color.working,
                if (live) MarkShape.Bar else MarkShape.Ring,
                6.dp,
            )
            KText("hub · ${hub?.name ?: "—"}", tokens.type.meta, if (live) tokens.color.done else tokens.color.working)
        }
        for (node in herd.nodes.filter { it.kind != "local" }) {
            KText("${node.name} ${formatLatency(node.rttMs)}", tokens.type.meta, tokens.color.mute)
        }
        Box(Modifier.weight(1f))
        val offline = connectionStatus as? ConnectionStatus.Offline
        val refused = connectionStatus is ConnectionStatus.Refused
        KText(
            when {
                refused -> "not paired — this node does not know this device"
                offline == null -> "no lease held — desktop shape untouched"
                else -> "reconnecting in ${offline.retryInMs / 1000}s — showing cached grid"
            },
            tokens.type.meta,
            if (refused) tokens.color.blocked else tokens.color.mute,
            when {
                refused -> Modifier.announce("Not paired — this node does not know this device")
                offline == null -> Modifier
                else -> Modifier.announce(
                    "Offline — reconnecting in ${offline.retryInMs / 1000} seconds, showing a cached grid",
                )
            },
        )
        KText("local ${formatLatency(localRtt)}", tokens.type.meta, tokens.color.mute)
        KText("kampr ${build ?: "0.1.0"}", tokens.type.meta, tokens.color.mute)
    }
}
