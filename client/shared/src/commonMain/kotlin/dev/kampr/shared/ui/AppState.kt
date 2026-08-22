package dev.kampr.shared.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.Enrolment
import dev.kampr.shared.net.AuthApi
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.net.InstallPrompt
import dev.kampr.shared.net.NodeApi
import dev.kampr.shared.net.PasskeyApi
import dev.kampr.shared.net.PasskeyOutcome
import dev.kampr.shared.net.createPasskeys
import dev.kampr.shared.net.createHttpClient
import dev.kampr.shared.net.createInstallPrompt
import dev.kampr.shared.net.defaultEndpoint
import dev.kampr.shared.net.deviceName
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
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

enum class PaneView(val key: String) {
    Terminal("terminal"),
    Conversation("conversation"),
    Split("split"),
}

fun viewOf(key: String): PaneView? = PaneView.entries.firstOrNull { it.key == key }

// Two stacks, and every screen is in one of them: the herd holds the list and whatever pane was
// opened out of it, settings holds the address, the pairing code, the machines, the devices,
// appearance and notifications. The desktop sidebar has called that second destination Settings
// since it was written; the phone called it Nodes and the operator could not tell what it was for.
//
// There was a third, "Pane", and it was not a peer of a list of everything: it led to whichever
// pane had last been opened, kept pointing at it after the pane had been left, and fell through to
// the herd when nothing had. A tab has to be a place.
enum class Tab { Herd, Settings }

fun screenFor(tab: Tab): Screen = when (tab) {
    Tab.Herd -> Screen.Herd
    Tab.Settings -> Screen.Setup
}

fun tabFor(screen: Screen): Tab = when (screen) {
    Screen.Setup, Screen.Devices, Screen.Appearance, Screen.Notifications -> Tab.Settings
    Screen.Herd, Screen.Mosaic, is Screen.Pane -> Tab.Herd
}

sealed interface Screen {
    data object Herd : Screen
    data object Mosaic : Screen
    data class Pane(val paneId: String, val view: PaneView) : Screen
    data object Setup : Screen
    data object Devices : Screen
    data object Appearance : Screen
    data object Notifications : Screen
}

// The herd-management surfaces, which float over whatever screen is showing rather than
// replacing it — everything they act on is behind them.
sealed interface Sheet {
    data class New(val nodeId: String, val paneId: String?) : Sheet
    data class Actions(val paneId: String) : Sheet
}

// One field carried both endings and one strip painted both, so a refusal arrived in the colour
// of a success — which on the screen about credentials is the worst version of getting it wrong.
data class PasskeyNote(val message: String, val refused: Boolean)

// Backing out of the system sheet is a decision, not a failure to report.
fun passkeyNoteOf(outcome: PasskeyOutcome): PasskeyNote? = when (outcome) {
    PasskeyOutcome.Cancelled -> null
    is PasskeyOutcome.Refused -> PasskeyNote(outcome.reason, refused = true)
    is PasskeyOutcome.Enrolled -> PasskeyNote("Passkey enrolled. This device now signs in with it.", refused = false)
}

private const val KEY_THEME = "theme"
private const val KEY_MODE = "mode"
private const val KEY_ENDPOINT = "endpoint"
private const val KEY_TOKEN = "token"
private const val KEY_DEVICE = "device"
private const val KEY_RECENT = "endpoints"
private const val KEY_AGENT_ARGS = "agent.args."

private const val RECENT_ADDRESSES = 5

class AppState(
    private val scope: CoroutineScope,
    val store: KamprStore = KamprStore(),
    val prefs: Prefs = createPrefs(),
    private val derived: Endpoint? = defaultEndpoint(),
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

    // A device with nothing to connect *with* opens on the screen that asks for it, rather than on
    // a herd it cannot fetch behind an error about a connection nobody asked it to make. An address
    // is not enough: the browser is served by the node, so it can always derive one, and a page that
    // has never paired would otherwise land on a herd that stays empty forever.
    var screen: Screen by mutableStateOf(if (endpoint?.token == null) Screen.Setup else Screen.Herd)
        private set

    var sheet: Sheet? by mutableStateOf(null)
        private set

    // The one thing that lets this device recognise itself in a list of devices, so it does not
    // offer to revoke the connection it is speaking over.
    var deviceId: String? by mutableStateOf(prefs.get(KEY_DEVICE))
        private set

    var pairingError: String? by mutableStateOf(null)
        private set

    // A passkey needs both halves to be true: an origin that can carry an RP ID, which
    // `hello.security.passkeys` answers, and an authenticator API, which only the platform can.
    private val passkeys = createPasskeys()

    private val install: InstallPrompt = createInstallPrompt()

    var installable: Boolean by mutableStateOf(false)
        private set

    val passkeysUsable: Boolean get() = passkeys.available

    var passkeyNote: PasskeyNote? by mutableStateOf(null)
        private set

    // Everything time-shaped here compares the node's clock against this device's: a pane's age is
    // stamped there and rendered here, and a snooze is computed here and filtered there.
    var clockOffsetMs: Double by mutableStateOf(0.0)
        private set

    val endpoint: Endpoint?
        get() {
            val saved = prefs.get(KEY_ENDPOINT) ?: return derived
            return Endpoint(saved, prefs.get(KEY_TOKEN))
        }

    // Per device rather than per node: it is a habit of whoever is holding the phone, and the
    // node's own `prefs` frame is keyed by pane, so it cannot hold a per-harness setting.
    val agentArgs: AgentArgs = PrefsAgentArgs(prefs)

    val recentAddresses: List<String>
        get() = prefs.get(KEY_RECENT).orEmpty().split('\n').filter { it.isNotBlank() }

    fun rememberAddress(url: String) {
        val kept = (listOf(url) + recentAddresses.filter { it != url }).take(RECENT_ADDRESSES)
        prefs.set(KEY_RECENT, kept.joinToString("\n"))
    }

    fun start() {
        watchInstallability()
        val target = endpoint ?: return
        // `useEndpoint` has refused to dial a tokenless address since 0.1.1 — "having nothing to
        // keep is not a reason to dial anyway and retry in silence". This is the other entry point,
        // and on the web it is the one that runs on every load.
        if (target.token == null) return
        push.prepare(target.token)
        connection.connect(target)
        warm(target)
    }

    fun install() {
        scope.launch {
            install.prompt()
            installable = install.available
        }
    }

    // `beforeinstallprompt` fires once, on the browser's own schedule, and often after wasm has
    // booted. Polling briefly is what turns "the event already went past" into an affordance.
    private fun watchInstallability() {
        scope.launch {
            repeat(30) {
                installable = install.available
                if (installable) return@launch
                delay(1_000)
            }
        }
    }

    fun useEndpoint(target: Endpoint) {
        scope.launch {
            pairingError = null
            val code = target.token?.trim()?.takeIf { it.isNotEmpty() }
            if (code == null) {
                // A blank code means "point at this address with what I already have". Treating it
                // as a token threw the enrolment away every time the address alone was edited —
                // and having nothing to keep is not a reason to dial anyway and retry in silence.
                val held = prefs.get(KEY_TOKEN)
                if (held == null) {
                    pairingError = "This device is not paired with that node yet. Type the pairing " +
                        "code it printed, or sign in with a passkey."
                    return@launch
                }
                adopt(target.copy(token = held), deviceId)
                return@launch
            }
            val enrolment = exchange(target, code) ?: return@launch
            adopt(target.copy(token = enrolment.token), enrolment.deviceId)
        }
    }

    fun dismissPairingError() {
        pairingError = null
    }

    fun dismissPasskeyNote() {
        passkeyNote = null
    }

    // Enrolling from an already-paired device: the node mints a second device around the new
    // credential, so what comes back is a token this client keeps using.
    fun enrolPasskey() {
        scope.launch {
            val target = endpoint ?: return@launch
            val outcome = withPasskeys(target) { it.enrol(deviceName()) }
            if (outcome is PasskeyOutcome.Enrolled) {
                adopt(target.copy(token = outcome.enrolment.token), outcome.enrolment.deviceId)
            }
            passkeyNote = passkeyNoteOf(outcome)
        }
    }

    // Signing in with one instead of typing a pairing code, which is the only enrolment path that
    // does not need somebody at the console.
    fun signInWithPasskey(target: Endpoint) {
        scope.launch {
            pairingError = null
            when (val outcome = withPasskeys(target.copy(token = null)) { it.signIn() }) {
                PasskeyOutcome.Cancelled -> {}
                is PasskeyOutcome.Refused -> pairingError = outcome.reason
                is PasskeyOutcome.Enrolled ->
                    adopt(target.copy(token = outcome.enrolment.token), outcome.enrolment.deviceId)
            }
        }
    }

    private suspend fun <T> withPasskeys(target: Endpoint, block: suspend (PasskeyApi) -> T): T {
        val client = createHttpClient()
        return try {
            block(PasskeyApi(client, target, passkeys))
        } finally {
            client.close()
        }
    }

    private fun adopt(resolved: Endpoint, device: String?) {
        deviceId = device
        prefs.set(KEY_DEVICE, device)
        prefs.set(KEY_ENDPOINT, resolved.baseUrl)
        prefs.set(KEY_TOKEN, resolved.token)
        rememberAddress(resolved.baseUrl)
        push.prepare(resolved.token)
        connection.connect(resolved)
        warm(resolved)
    }

    // A refused code is a refusal. Handing the typed code to the socket as a bearer is what turned
    // one mistyped character into an endless `auth.rejected` loop with nothing at all on screen.
    private suspend fun exchange(target: Endpoint, code: String): Enrolment? {
        val client = createHttpClient()
        val enrolment = try {
            AuthApi(client, target).pair(code, deviceName())
        } finally {
            client.close()
        }
        if (enrolment == null) {
            pairingError = "That pairing code was not accepted. Codes expire after ten minutes, " +
                "and one printed at a console needs a keypress there before it works."
        }
        return enrolment
    }

    // The service worker's warm cache is written behind every push and was read by nobody: the
    // page never asked for either URL it holds. Asking is what turns a tap on a notification into
    // a herd that is already painted when the socket finishes opening.
    private fun warm(target: Endpoint) {
        if (target.token == null) return
        scope.launch {
            val client = createHttpClient()
            val body = try {
                val api = NodeApi(client, target)
                api.clockOffsetMillis()?.let { clockOffsetMs = it }
                api.warm()
            } finally {
                client.close()
            }
            // Only ahead of the socket, never over it: the live herd is the truth the moment it
            // arrives, and a cached copy landing late would be a herd going backwards.
            if (body != null && !store.herd.value.known) {
                Wire.decode(body)?.let(store::accept)
            }
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

    fun openSheet(which: Sheet) {
        if (!store.canManage) return
        // A stale ack from an earlier op would otherwise close the sheet the moment it opened.
        store.clearManaged()
        sheet = which
    }

    fun closeSheet() {
        sheet = null
    }

    fun manage(op: ManageOp) {
        connection.manage(op)
    }

    // Watch on arrival, unwatch on departure: an observer that outlives the screen holding it is
    // exactly the thing the mosaic's observer count would then be lying about.
    fun go(target: Screen) {
        sheet = null
        val leaving = (screen as? Screen.Pane)?.paneId
        if (target is Screen.Pane) connection.watch(target.paneId)
        if (leaving != null && leaving != (target as? Screen.Pane)?.paneId) connection.unwatch(leaving)
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

    fun selectTab(tab: Tab) = go(screenFor(tab))

    fun back() {
        go(Screen.Herd)
    }
}

private class PrefsAgentArgs(private val prefs: Prefs) : AgentArgs {
    override fun get(kind: String): String = prefs.get(KEY_AGENT_ARGS + kind).orEmpty()
    override fun remember(kind: String, text: String?) = prefs.set(KEY_AGENT_ARGS + kind, text)
}
