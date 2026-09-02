package dev.kampr.shared.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.createdPane
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.Enrolment
import dev.kampr.shared.net.AuthApi
import dev.kampr.shared.net.Pairing
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.net.InstallPrompt
import dev.kampr.shared.net.AttachmentApi
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.NodeApi
import dev.kampr.shared.net.PasskeyApi
import dev.kampr.shared.net.PushApi
import dev.kampr.shared.net.PasskeyOutcome
import dev.kampr.shared.net.createPasskeys
import dev.kampr.shared.net.createHttpClient
import dev.kampr.shared.net.createInstallPrompt
import dev.kampr.shared.net.defaultEndpoint
import dev.kampr.shared.net.deviceName
import dev.kampr.shared.model.SeenDone
import dev.kampr.shared.model.unreadDone
import dev.kampr.shared.platform.Prefs
import dev.kampr.shared.platform.createPrefs
import dev.kampr.shared.push.PushPlatform
import dev.kampr.shared.push.createPushPlatform
import dev.kampr.shared.theme.ThemeId
import dev.kampr.shared.theme.ThemeMode
import dev.kampr.shared.theme.ThemeSpec
import dev.kampr.shared.theme.modeOf
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.net.wallClockMillis
import dev.kampr.shared.model.fleetTargets
import dev.kampr.shared.model.newCohortId
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.Wire
import dev.kampr.shared.wire.talks
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.MutableStateFlow
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
    Screen.Herd, Screen.Mosaic, Screen.Fleet, is Screen.Pane -> Tab.Herd
}

sealed interface Screen {
    data object Herd : Screen
    data object Mosaic : Screen
    // One command across the herd, grouped by the fan-out that produced it. A place of its own
    // rather than a section of the herd: a fleet run is on nobody's desk, and "which host needs
    // me" is not the herd's question.
    data object Fleet : Screen
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

    // The list menu, off a row rather than off a screen. `at` is where it hangs from on a desk and
    // null on a phone, which has no pointer and gets the bottom sheet instead.
    data class Menu(val paneId: String, val at: MenuAnchor?) : Sheet
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
private const val KEY_RAIL = "sidebar.collapsed"

private const val RECENT_ADDRESSES = 5

// How long a create op's ack stays worth acting on. Generous on purpose: it is the window in which
// an *unrelated* patch may land in front of the one carrying the pane, and the cost of overrunning
// it is that the operator lands on the herd, which is where they were.
private const val CREATE_OPEN_WINDOW_MS = 15_000.0

class AppState(
    private val scope: CoroutineScope,
    val store: KamprStore = KamprStore(),
    val prefs: Prefs = createPrefs(),
    private val derived: Endpoint? = defaultEndpoint(),
    // The service worker outlives every page and cannot read where the token is kept, so it is
    // handed over on the way past. Without it a push arrives, warms nothing, and the tap that
    // follows loads from cold.
    val push: PushPlatform = createPushPlatform(),
) {
    val connection = KamprConnection(scope, store)

    // Which pane, if any, this client is holding a controller on. One at a time: the panel that
    // starts a hold releases the previous one, and the node refuses a second controller anyway
    // (#21). Kept here rather than on the pane's own state because the status strip is what has to
    // stop claiming the desk is untouched.
    private val held = MutableStateFlow<String?>(null)
    val heldPane: StateFlow<String?> = held

    fun holdingPane(paneId: String, holding: Boolean) {
        held.value = when {
            holding -> paneId
            held.value == paneId -> null
            else -> held.value
        }
    }

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

    // Survives a reload because the browser is where this client is most often left open: an
    // operator who collapsed the sidebar to read a pane has not asked for it back on every refresh.
    var sidebarCollapsed: Boolean by mutableStateOf(prefs.get(KEY_RAIL) == "1")
        private set

    fun collapseSidebar(collapsed: Boolean) {
        sidebarCollapsed = collapsed
        prefs.set(KEY_RAIL, if (collapsed) "1" else null)
    }

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
        announcePush(target)
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
        announcePush(resolved)
        connection.connect(resolved)
        warm(resolved)
    }

    // A refused code is a refusal. Handing the typed code to the socket as a bearer is what turned
    // one mistyped character into an endless `auth.rejected` loop with nothing at all on screen.
    private suspend fun exchange(target: Endpoint, code: String): Enrolment? {
        val client = createHttpClient()
        val outcome = try {
            AuthApi(client, target).pair(code, deviceName())
        } finally {
            client.close()
        }
        pairingError = when (outcome) {
            is Pairing.Enrolled -> null
            Pairing.Refused ->
                "That pairing code was not accepted. Codes expire after ten minutes, " +
                    "and one printed at a console needs a keypress there before it works."
            Pairing.Busy -> "That node is busy. The code is still good — try again in a moment."
            Pairing.Unreachable -> "Could not reach that node. The code is still good."
        }
        return (outcome as? Pairing.Enrolled)?.enrolment
    }

    // A one-shot client for a one-shot fetch, the same shape the pairing and warm calls use: an
    // attachment is asked for by a press, and holding a connection open between presses buys
    // nothing on a link this is deliberately kept off.
    suspend fun fetchAttachment(paneId: String, id: String): AttachmentBytes {
        val target = endpoint ?: return AttachmentBytes.Failed("This device is not connected to a node.")
        val client = createHttpClient()
        return try {
            AttachmentApi(client, target).fetch(paneId, id)
        } finally {
            client.close()
        }
    }

    // The service worker's warm cache is written behind every push and was read by nobody: the
    // page never asked for either URL it holds. Asking is what turns a tap on a notification into
    // a herd that is already painted when the socket finishes opening.
    // Re-states a subscription this device already holds, so the node's record of what this
    // client can read is this build's and not the one it first subscribed under. Idempotent — the
    // node upserts on the endpoint — and silent: a device with no subscription has nothing to say
    // and says nothing.
    //
    // Without it a phone that subscribed under an older build goes on receiving only the kinds
    // that build declared, with nothing anywhere saying why the rest never arrive.
    private fun announcePush(target: Endpoint) {
        if (target.token == null) return
        scope.launch {
            val enrolment = push.enrolment() ?: return@launch
            val client = createHttpClient()
            try {
                PushApi(client, target).subscribe(enrolment)
            } finally {
                client.close()
            }
        }
    }

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

    // An answer to a fleet host, typed as the ordinary pane input every other reply uses. There is
    // no `fleet.answer` and there should not be: a second way to type into a terminal is a second
    // thing to get wrong.
    fun answerFleet(paneId: String, text: String) {
        connection.send(ClientMsg.InputText(paneId, text + "\n"))
    }

    // One command, one op per machine that can be reached, all sharing a cohort so the board can
    // gather them. The cohort is minted here because a run spans hosts and no single node can name
    // one.
    fun runFleet(argv: List<String>) {
        if (argv.isEmpty()) return
        val targets = fleetTargets(store.herd.value.nodes)
        if (targets.isEmpty()) return
        val cohort = newCohortId(wallClockMillis().toLong())
        targets.forEach { node ->
            connection.manage(ManageOp.FleetRun(node = node.id, cohort = cohort, args = argv))
        }
        go(Screen.Fleet)
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

    // True while the pane on screen is showing a default nobody asked for. `prefs` is the third
    // frame of the greeting and lands after `herd`, so anything that opens a pane the moment the
    // herd arrives reads a memory that is not there yet — and without this the guess would stand
    // for the rest of the session, which is the whole of "it forgot what I picked".
    private var awaitingRemembered = false

    // What this device last chose for this pane, and the terminal when it has chosen nothing. The
    // old default read the pane instead — an agent with no ring opened in Conversation — and a
    // ring of zero says only that there is no history above the grid, not that there is no grid:
    // the observe stream paints one either way, and every live Claude pane reports zero because
    // the harness clears the scrollback when it takes the screen (#30, #231). So that branch was
    // spending the operator's own choice on a guess about a pane with something to show regardless.
    // Whether this client is drawing at phone size. Set by the composable that knows the window,
    // and the only thing that moves the default view: a 213-column grid on a 411 dp screen is not
    // a terminal anybody can use, and a conversation is. On a desktop — and in the CLI, which made
    // the same change for the same reason — the terminal is what a terminal client opens on.
    var compact: Boolean = false

    // Opening the pane is what marks its `done` read. It is the only trigger, and it is local:
    // clearing herdr's own marker would take a focus op, which is the operator's press.
    val seenDone = SeenDone(prefs)

    fun openPane(paneId: String, prefer: PaneView? = null) {
        seenDone.saw(store.paneInfo(paneId))
        // Opening the pane *is* the read, so the notification standing for it comes down now
        // rather than at the next herd update — which is up to a poll interval away, and is a
        // notification for something the operator is looking at.
        reconcileNotifications(store.herd.value)
        val remembered = store.prefsFor(paneId).view?.let(::viewOf)
        awaitingRemembered = prefer == null && remembered == null
        go(Screen.Pane(paneId, prefer ?: remembered ?: defaultViewOf(paneId)))
    }

    // The operator's own choice always wins; this is only what a pane nobody has chosen for opens
    // on. `converses` rather than `hasConversation`: a session that opened a minute ago has an
    // adapter and no transcript file, and it is exactly the one somebody is about to talk to.
    private fun defaultViewOf(paneId: String): PaneView {
        if (!compact) return PaneView.Terminal
        val pane = store.herd.value.panes.firstOrNull { it.id == paneId } ?: return PaneView.Terminal
        return when (pane.talks) {
            true -> PaneView.Conversation
            false -> PaneView.Terminal
        }
    }

    private fun adoptRememberedView() {
        if (!awaitingRemembered) return
        val current = screen as? Screen.Pane ?: return
        val view = store.prefsFor(current.paneId).view?.let(::viewOf) ?: return
        awaitingRemembered = false
        // Not `go`: the pane is the one already being watched, and re-entering it would churn the
        // watch. Not `setPaneView` either — this is the node's own memory arriving, not a write.
        screen = current.copy(view = view)
    }

    fun setPaneView(view: PaneView) {
        val current = screen
        if (current !is Screen.Pane) return
        awaitingRemembered = false
        screen = current.copy(view = view)
        connection.send(ClientMsg.SetPrefs(current.paneId, mapOf("view" to view.key)))
    }

    fun selectTab(tab: Tab) = go(screenFor(tab))

    fun back() {
        go(Screen.Herd)
    }

    // A notification the node sent is a summary of the moment it sent it. This client sees the
    // herd move first-hand, so a prompt answered anywhere else comes down here without waiting for
    // a push to say so — which is the case the node cannot help with at all when the phone was
    // asleep and the answer happened at the desk.
    //
    // `known` is the guard that matters: an unloaded herd has no blocked panes either, and
    // reconciling against it would take down the very notification whose tap opened the app.
    //
    // The finished half is reconciled against what this device has *read* rather than against
    // herdr's own marker: only focusing the pane at the desk clears that (#357, #396), and
    // opening it here deliberately does not (rule 3). So a finish read in this app comes off this
    // phone without anything being written back to the node.
    // What a create op was told it made, held until the pane inside it turns up.
    //
    // A `managed` ack answers on the socket before the sweep that finds the pane, so there is
    // nothing to open at the moment the sheet closes — which is why a new workspace used to appear
    // at the foot of the herd and stay there. Nothing here writes to herdr: opening a pane is a
    // watch and a screen, never a `focus` (rule 3).
    private var creating: Pair<String, Double>? = null

    // Held for a bounded time rather than until the next patch. A structural op is **not** settled
    // before its ack — only the session ops are, and they reconcile the herd first (`spawn_settle`)
    // — so the pane arrives on whatever sweep or `workspace.created` event notices it, and an
    // unrelated patch can easily land in front of it. Held for ever is the other failure: these ids
    // come round again, and an intent nobody cancelled would one day open a pane the operator did
    // not ask for.
    fun opening(id: String?) {
        creating = id?.let { it to wallClockMillis() + CREATE_OPEN_WINDOW_MS }
    }

    private fun openWhenItArrives(herd: Herd) {
        val (wanted, deadline) = creating ?: return
        if (!herd.known) return
        val pane = herd.createdPane(wanted)
        if (pane == null) {
            if (wallClockMillis() > deadline) creating = null
            return
        }
        creating = null
        openPane(pane.id)
    }

    private fun reconcileNotifications(herd: Herd) {
        if (!herd.known) return
        // A pane on the screen is a pane being read. Two cases need it here rather than in
        // `openPane`: one opened from a notification *before* the herd arrived, which `openPane`
        // had nothing to mark; and one already open when the agent in it finishes, which is
        // somebody watching it happen. herdr can say `done` for either (#399), and a notification
        // about the pane already filling the screen is the app talking to itself.
        (screen as? Screen.Pane)?.let { seenDone.saw(store.paneInfo(it.paneId)) }
        push.reconcile(store.blocked().isNotEmpty(), herd.unreadDone(seenDone).isNotEmpty())
    }

    // Last in the class on purpose: an Unconfined collector runs its first emission inside the
    // constructor, and `adoptRememberedView` reads `screen`.
    init {
        scope.launch { store.prefs.collect { adoptRememberedView() } }
        scope.launch {
            store.herd.collect {
                openWhenItArrives(it)
                reconcileNotifications(it)
            }
        }
    }
}

private class PrefsAgentArgs(private val prefs: Prefs) : AgentArgs {
    override fun get(kind: String): String = prefs.get(KEY_AGENT_ARGS + kind).orEmpty()
    override fun remember(kind: String, text: String?) = prefs.set(KEY_AGENT_ARGS + kind, text)
}
