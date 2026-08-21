package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.collectAsState
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.net.ALL_PANES
import dev.kampr.shared.net.PushApi
import dev.kampr.shared.net.PushRule
import dev.kampr.shared.net.PushState
import dev.kampr.shared.net.createHttpClient
import dev.kampr.shared.net.wallClockMillis
import dev.kampr.shared.push.PushCapability
import dev.kampr.shared.push.PushPermission
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import kotlinx.coroutines.launch

private const val SNOOZE_MINUTES = 60

// The store, for the few surfaces that need to *observe* rather than send — `PaneIo` is a
// one-way channel by design and widening it would put a reply flow on every pane surface.
val LocalKamprStore: ProvidableCompositionLocal<KamprStore?> = staticCompositionLocalOf { null }

// Everything about notifications on this device, and — where they cannot work — what would make
// them work instead of a control that fails at the last step.
@Composable
fun NotificationsScreen(
    state: AppState,
    panes: List<PaneInfo>,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val scope = rememberCoroutineScope()
    var server by remember { mutableStateOf<PushState?>(null) }
    var busy by remember { mutableStateOf(false) }
    var note by remember { mutableStateOf<String?>(null) }
    val capability = remember { state.push.capability() }
    val nodeClock = wallClockMillis() + state.clockOffsetMs

    suspend fun reload() {
        val target = state.endpoint ?: return
        val client = createHttpClient()
        try {
            server = PushApi(client, target).state()
        } finally {
            client.close()
        }
    }

    LaunchedEffect(state.endpoint?.baseUrl) { reload() }

    fun act(run: suspend (PushApi) -> Unit) {
        if (busy) return
        val target = state.endpoint ?: return
        busy = true
        scope.launch {
            val client = createHttpClient()
            try {
                run(PushApi(client, target))
                server = PushApi(client, target).state()
            } finally {
                client.close()
                busy = false
            }
        }
    }

    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        Row(
            Modifier.fillMaxWidth().padding(start = 16.dp, top = 16.dp, end = 20.dp, bottom = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(11.dp),
        ) {
            BackAction("Back", onBack)
            KText("Notifications", tokens.type.screenTitle, tokens.color.text, Modifier.weight(1f).asHeading())
        }
        Column(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).widthIn(max = 620.dp)
                .padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(11.dp),
        ) {
            val blocker = blocker(capability, server)
            if (blocker != null) {
                Surface(Modifier.fillMaxWidth()) {
                    Column(
                        Modifier.padding(15.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        KText(blocker.first, tokens.type.bodyStrong, tokens.color.text)
                        KText(blocker.second, tokens.type.caption, tokens.color.dim)
                    }
                }
            } else {
                val subscribed = server?.subscribed == true
                Surface(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(15.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        KText(
                            if (subscribed) "This device is notified" else "Get told when an agent is blocked",
                            tokens.type.bodyStrong,
                            tokens.color.text,
                        )
                        KText(
                            if (subscribed) {
                                "A blocked agent wakes this device, with its question in the notification."
                            } else {
                                "The question comes with it, so you can decide before the app opens."
                            },
                            tokens.type.caption,
                            tokens.color.dim,
                        )
                        if (subscribed) {
                            QuietAction("Stop notifying this device", {
                                // Unsubscribing in the browser first, so a node that is unreachable
                                // cannot leave a live subscription this device no longer wants.
                                scope.launch {
                                    val endpoint = state.push.unsubscribe()
                                    if (endpoint != null) act { api -> api.unsubscribe(endpoint) } else reload()
                                }
                            }, Modifier.fillMaxWidth())
                        } else {
                            // Browsers only honour a permission request inside the gesture that
                            // started it, which is why this is a button and never an effect.
                            PrimaryAction("Turn on notifications", {
                                val key = server?.key ?: return@PrimaryAction
                                scope.launch {
                                    val enrolment = state.push.subscribe(key)
                                    if (enrolment == null) {
                                        note = "The browser did not grant permission."
                                        return@launch
                                    }
                                    act { api -> api.subscribe(enrolment) }
                                }
                            }, Modifier.fillMaxWidth())
                        }
                        note?.let { KText(it, tokens.type.meta, tokens.color.blocked, Modifier.announce(it)) }
                    }
                }

                RuleRow(
                    title = "Every agent",
                    subtitle = "Mute or snooze the whole herd on this device",
                    rule = server?.rules?.firstOrNull { it.paneId == ALL_PANES },
                    now = nodeClock,
                    onMute = { muted -> act { it.rule(PushRule(ALL_PANES, muted = muted)) } },
                    onSnooze = { act { it.rule(PushRule(ALL_PANES, snoozeUntil = snoozeUntil(nodeClock))) } },
                )

                val agents = panes.filter { it.agent != null }
                if (agents.isNotEmpty()) {
                    LabelText(
                        "Per agent",
                        tokens.type.sectionLabel,
                        tokens.color.dim,
                        Modifier.padding(top = 6.dp),
                    )
                }
                for (pane in agents) {
                    RuleRow(
                        title = paneTitle(pane),
                        subtitle = pane.cwd ?: pane.id,
                        rule = server?.rules?.firstOrNull { it.paneId == pane.id },
                        now = nodeClock,
                        onMute = { muted -> act { it.rule(PushRule(pane.id, muted = muted)) } },
                        onSnooze = { act { it.rule(PushRule(pane.id, snoozeUntil = snoozeUntil(nodeClock))) } },
                    )
                }
            }
            Box(Modifier.padding(bottom = 18.dp))
        }
    }
}

@Composable
private fun RuleRow(
    title: String,
    subtitle: String,
    rule: PushRule?,
    now: Double,
    onMute: (Boolean) -> Unit,
    onSnooze: () -> Unit,
) {
    val tokens = Kampr.tokens
    val muted = rule?.muted == true
    val snoozing = rule?.snoozeUntil?.let { it * 1000.0 > now } == true
    Surface(Modifier.fillMaxWidth()) {
        Row(
            Modifier.padding(horizontal = 15.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                KText(title, tokens.type.bodyStrong, tokens.color.text)
                KText(
                    when {
                        muted -> "Muted"
                        snoozing -> "Snoozed"
                        else -> subtitle
                    },
                    tokens.type.meta,
                    if (muted || snoozing) tokens.color.working else tokens.color.mute,
                )
            }
            if (!muted && !snoozing) {
                QuietAction(
                    "Snooze ${SNOOZE_MINUTES}m", onSnooze, Modifier.widthIn(min = 96.dp),
                    label = "Snooze $title for $SNOOZE_MINUTES minutes",
                )
            }
            QuietAction(
                if (muted || snoozing) "Unmute" else "Mute",
                { onMute(!muted && !snoozing) },
                Modifier.widthIn(min = 84.dp),
                label = if (muted || snoozing) "Unmute $title" else "Mute $title",
            )
        }
    }
}

// The node filters a snooze on its own clock (`kampr-auth/src/push.rs`), so a deadline taken from
// the device's is wrong by exactly the skew, in whichever direction hurts.
private fun snoozeUntil(nodeClock: Double): Long = (nodeClock / 1000).toLong() + SNOOZE_MINUTES * 60L

// The single place that decides whether a subscribe control may exist at all. Each branch says
// what would unlock it rather than leaving the user to guess, which is the whole of findings §3.7
// applied to one screen.
private fun blocker(capability: PushCapability, server: PushState?): Pair<String, String>? = when {
    capability is PushCapability.NeedsHomeScreen -> Pair(
        "Add Kampr to your Home Screen first",
        "iOS grants notifications only to a Home Screen web app. Tap Share, then Add to Home " +
            "Screen, and open Kampr from the icon — notifications appear here once it is installed.",
    )
    capability is PushCapability.InsecureContext || server?.secureContext == false -> Pair(
        "This address cannot do notifications",
        "Notifications need a secure context, and plain HTTP on an IP address is not one. Point a " +
            "hostname at this machine and give it a certificate — the same step that unlocks passkeys.",
    )
    capability is PushCapability.NeedsDistributor -> Pair(
        "Install a UnifiedPush distributor",
        "Android has no push service that does not go through Google, so Kampr uses UnifiedPush: " +
            "a small app you install once and point wherever you like — ntfy is the usual choice, " +
            "and it can talk to your own ntfy server. Install one, then come back.",
    )
    capability is PushCapability.Unsupported -> Pair(
        "This build has no notification channel",
        "The desktop app is always on screen, and the Android app uses UnifiedPush rather than the " +
            "browser's push service.",
    )
    server == null -> Pair("Asking the node…", "Waiting for this node to say what it can do.")
    !server.available -> Pair(
        "This node is not sending notifications",
        "It has no VAPID key, or push is switched off in its configuration.",
    )
    capability is PushCapability.Ready && capability.permission == PushPermission.Denied -> Pair(
        "Notifications are blocked for this site",
        "The browser is refusing them. Allow notifications for this address in its site settings, " +
            "then come back.",
    )
    else -> null
}

// "I'm taking this pane" — probe #50 pointed at the one place it is not noise.
//
// A second person may be sitting at the desk this pane belongs to, and the thing worth telling
// them is that somebody remote is about to type into it. The node attributes the toast to this
// device, so the desk always sees who; a read-only device is refused, so it cannot.
@Composable
fun TakingPaneAction(paneId: String, title: String, modifier: Modifier = Modifier) {
    val io = LocalPaneIo.current
    val store = LocalKamprStore.current
    if (io.readOnly || store == null) return
    var asked by remember(paneId) { mutableStateOf(false) }
    val reply by store.notified.collectAsState()
    // A headless herdr has no desk to show a toast on and says so (probe #77). Reporting "told"
    // regardless would be the client inventing an outcome the node explicitly denied.
    val label = when {
        !asked -> "Tell the desk"
        reply == null -> "Telling…"
        reply?.ok == true -> "Desk told"
        else -> "No desk"
    }
    QuietAction(
        label,
        {
            store.clearNotified()
            asked = true
            io.send(ClientMsg.Notify("Taking $title", "from a Kampr device", paneId))
        },
        modifier,
    )
}
