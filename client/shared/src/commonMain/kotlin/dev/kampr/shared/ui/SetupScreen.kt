package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.PairingScanSurface
import dev.kampr.shared.net.SetupStatus
import dev.kampr.shared.net.pairingFrom
import dev.kampr.shared.net.pairingScanAvailable
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.util.joinLink
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.Security

private data class Rung(
    val icon: Icon,
    val title: String,
    val detail: String,
    val unlocks: List<String> = emptyList(),
    val lead: Boolean = false,
    val onClick: (() -> Unit)? = null,
)

private val unlockCopy = mapOf(
    "passkeys" to "unlocks passkeys",
    "push" to "notifications",
    "installable" to "install to home screen",
)

// The ladder is built from hello.security, never from the URL: a rung whose affordance cannot
// work on this node is absent rather than present-and-failing.
private fun ladderFor(
    security: Security,
    onPasskeys: (() -> Unit)?,
    onInstall: (() -> Unit)?,
): List<Rung> = buildList {
    // Offered only when the browser has actually deferred a prompt. `security.installable` alone
    // said yes at tier 0 with nothing behind it, which is the checklist promising a capability
    // the client did not have.
    if (onInstall != null) {
        add(
            Rung(
                KamprIcons.nodes,
                "Install to the home screen",
                "It opens without browser chrome, and on iOS it is the only way notifications work.",
                lead = true,
                onClick = onInstall,
            )
        )
    }
    // `onPasskeys` is the effective answer: the socket may not be up yet, and `/api/node` says the
    // same thing without a token.
    if (security.passkeys || onPasskeys != null) {
        add(
            Rung(
                KamprIcons.lock,
                "Add a passkey",
                "This node is reachable at a name with a certificate, so WebAuthn works here.",
                lead = true,
                onClick = onPasskeys,
            )
        )
    } else {
        add(
            Rung(
                KamprIcons.lock,
                "A hostname and certificate",
                "Point your reverse proxy at this port. Nginx Proxy Manager, Caddy, Traefik — anything.",
                security.unlocks.mapNotNull(unlockCopy::get),
                lead = true,
            )
        )
    }
    if (security.tier < 3) {
        add(
            Rung(
                KamprIcons.globe,
                "Reach it from anywhere",
                "Tailscale, or your own domain. Kampr does not care which — it never assumes a tailnet.",
            )
        )
    }
    add(
        Rung(
            KamprIcons.nodes,
            "Add another machine",
            "Run Kampr there, paste one join code. It dials out, so it needs no open port.",
        )
    )
}

@Composable
fun SetupScreen(
    status: SetupStatus?,
    security: Security,
    running: Boolean,
    endpoint: Endpoint?,
    nodes: List<NodeInfo>,
    pairingCode: String?,
    pairingError: String?,
    onConnect: (Endpoint) -> Unit,
    onPairingCode: () -> Unit,
    onOpenHerd: () -> Unit,
    onDevices: () -> Unit,
    onAppearance: () -> Unit,
    onNotifications: () -> Unit,
    onPasskeys: (() -> Unit)? = null,
    onPasskeySignIn: ((Endpoint) -> Unit)? = null,
    onInstall: (() -> Unit)? = null,
    recentAddresses: List<String> = emptyList(),
    offeredCode: String? = null,
    wide: Boolean = false,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    var scanning by remember { mutableStateOf(false) }
    // What the camera read, kept so a refused enrolment leaves the address and the code on screen
    // to be corrected rather than retyped from memory.
    var scanned by remember { mutableStateOf<Endpoint?>(null) }
    Box(Modifier.fillMaxSize()) {
        Column(modifier.fillMaxSize().background(tokens.color.bg)) {
            Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                Column(
                    Modifier.widthIn(max = 520.dp).padding(start = 22.dp, top = 22.dp, end = 22.dp),
                    verticalArrangement = Arrangement.spacedBy(9.dp),
                ) {
                    Row(
                        Modifier.announce(if (running) "This node is running" else "This node is not reachable"),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(9.dp),
                    ) {
                        Mark(
                            if (running) tokens.color.done else tokens.color.blocked,
                            if (running) MarkShape.Bar else MarkShape.Square,
                            9.dp,
                        )
                        LabelText(
                            if (running) "Running" else "Not reachable",
                            tokens.type.caption.copy(fontWeight = tokens.label.weight, letterSpacing = tokens.label.tracking),
                            if (running) tokens.color.done else tokens.color.blocked,
                        )
                    }
                    KText(
                        if (running) "You're already in." else "Point Kampr at a node.",
                        tokens.type.screenTitle,
                        tokens.color.text,
                        Modifier.asHeading(),
                        maxLines = 2,
                    )
                    KText(
                        "Nothing to configure. Everything below is optional, and each one says what it buys you.",
                        tokens.type.body,
                        tokens.color.dim,
                        maxLines = 3,
                    )
                }

                if (!running || pairingError != null) {
                    Box(Modifier.widthIn(max = 520.dp).padding(start = 18.dp, top = 16.dp, end = 18.dp)) {
                        ConnectPanel(
                            scanned ?: endpoint,
                            pairingError,
                            onConnect,
                            onPasskeySignIn,
                            recentAddresses,
                            scanned?.token ?: offeredCode,
                            onScan = { scanning = true }.takeIf { pairingScanAvailable },
                        )
                    }
                }

                Box(Modifier.widthIn(max = 520.dp).padding(start = 18.dp, top = 16.dp, end = 18.dp)) {
                    Surface(Modifier.fillMaxWidth()) {
                        Column(
                            Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
                            verticalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(11.dp),
                            ) {
                                PairingMark()
                                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                                    KText(
                                        status?.address ?: endpoint?.baseUrl ?: "no node yet",
                                        tokens.type.meta.copy(fontSize = tokens.type.cardTitle.fontSize),
                                        tokens.color.text,
                                        maxLines = 2,
                                    )
                                    KText(
                                        pairingCode?.let { "pair with $it" }
                                            ?: "type this address on the other device",
                                        tokens.type.captionSmall,
                                        if (pairingCode != null) tokens.color.done else tokens.color.mute,
                                    )
                                }
                                if (running) {
                                    QuietAction(
                                        if (pairingCode != null) "New code" else "Pair a device",
                                        onPairingCode,
                                        Modifier.widthIn(min = 104.dp),
                                        label = "Print a pairing code another device can redeem",
                                    )
                                }
                            }
                            // The QR is for the *other* device: this one already has the address in
                            // its own address bar. A phone-width portrait layout is that other device,
                            // so it gets the code as text and no picture of where it already is.
                            val origin = status?.address ?: endpoint?.baseUrl
                            if (wide && origin != null) {
                                PairingQr(joinLink(origin, pairingCode), pairingCode != null)
                            }
                            // A code this device asked for is armed by construction, unlike one
                            // printed at a console — which is the whole reason this is worth having.
                            if (pairingCode != null) {
                                KText(
                                    "Good for ten minutes and one device. It works as it stands: " +
                                        "you asked for it from a device that is already trusted.",
                                    tokens.type.captionSmall,
                                    tokens.color.dim,
                                    maxLines = 3,
                                )
                            }
                            if (security.unencryptedBanner) {
                                val shape = RoundedCornerShape(tokens.radii.sm)
                                Row(
                                    Modifier
                                        .fillMaxWidth()
                                        .background(tokens.color.blockedBg, shape)
                                        .border(1.dp, tokens.color.blocked, shape)
                                        .announce(
                                            "Warning: plain HTTP on your LAN. Fine to try; add a " +
                                                "certificate before you leave it running.",
                                        )
                                        .padding(horizontal = 11.dp, vertical = 9.dp),
                                    horizontalArrangement = Arrangement.spacedBy(9.dp),
                                ) {
                                    IconGlyph(KamprIcons.warning, 14.dp, tokens.color.blocked)
                                    KText(
                                        "Plain HTTP on your LAN. Fine to try; add a certificate before you leave it running.",
                                        tokens.type.captionSmall,
                                        tokens.color.dim,
                                        maxLines = 3,
                                    )
                                }
                            }
                        }
                    }
                }

                Box(Modifier.padding(start = 22.dp, top = 20.dp, end = 22.dp, bottom = 9.dp)) {
                    LabelText("Optional, in any order", tokens.type.captionSmall, tokens.color.mute)
                }

                Column(
                    Modifier.widthIn(max = 520.dp).padding(horizontal = 18.dp),
                    verticalArrangement = Arrangement.spacedBy(9.dp),
                ) {
                    for (rung in ladderFor(security, onPasskeys, onInstall)) RungCard(rung)
                }

                Box(Modifier.padding(start = 22.dp, top = 20.dp, end = 22.dp, bottom = 9.dp)) {
                    LabelText("Machines in this herd", tokens.type.captionSmall, tokens.color.mute)
                }
                Column(
                    Modifier.widthIn(max = 520.dp).padding(horizontal = 18.dp),
                    verticalArrangement = Arrangement.spacedBy(9.dp),
                ) {
                    if (nodes.isEmpty()) {
                        KText(
                            "No machines yet. `kampr mesh invite` on this one, `kampr mesh join` on the other.",
                            tokens.type.captionSmall,
                            tokens.color.mute,
                            Modifier.padding(horizontal = 4.dp),
                            maxLines = 3,
                        )
                    }
                    for (machine in nodes) MachineCard(machine)
                }

                Box(Modifier.padding(start = 22.dp, top = 20.dp, end = 22.dp, bottom = 9.dp)) {
                    LabelText("This device", tokens.type.captionSmall, tokens.color.mute)
                }
                Box(Modifier.widthIn(max = 520.dp).padding(horizontal = 18.dp, vertical = 0.dp)) {
                    Surface(Modifier.fillMaxWidth().touchable().action("Devices paired with this node", onDevices)) {
                        Row(
                            Modifier.padding(horizontal = 15.dp, vertical = 13.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            Badge(34.dp, 17.dp, KamprIcons.lock, tokens.color.dim)
                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                KText("Devices", tokens.type.bodyStrong, tokens.color.text)
                                KText(
                                    status?.let { "${it.devices} paired · tier ${security.tier}" } ?: "tier ${security.tier}",
                                    tokens.type.captionSmall,
                                    tokens.color.dim,
                                )
                            }
                            IconGlyph(KamprIcons.chevronRight, 13.dp, tokens.color.mute)
                        }
                    }
                }
                Box(Modifier.widthIn(max = 520.dp).padding(horizontal = 18.dp, vertical = 9.dp)) {
                    Surface(Modifier.fillMaxWidth().touchable().action("Appearance — themes and ground", onAppearance)) {
                        Row(
                            Modifier.padding(horizontal = 15.dp, vertical = 13.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            Badge(34.dp, 17.dp, KamprIcons.gear, tokens.color.dim)
                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                KText("Appearance", tokens.type.bodyStrong, tokens.color.text)
                                KText("Four skins, light or dark ground", tokens.type.captionSmall, tokens.color.dim)
                            }
                            IconGlyph(KamprIcons.chevronRight, 13.dp, tokens.color.mute)
                        }
                    }
                }
                // Hidden where it cannot work, rather than present and failing at the last step: push
                // needs a secure context and `hello.security` is what says whether this origin is one.
                if (security.push) {
                    Box(Modifier.widthIn(max = 520.dp).padding(horizontal = 18.dp, vertical = 9.dp)) {
                        Surface(Modifier.fillMaxWidth().touchable().action("Notifications on this device", onNotifications)) {
                            Row(
                                Modifier.padding(horizontal = 15.dp, vertical = 13.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(12.dp),
                            ) {
                                Badge(34.dp, 17.dp, KamprIcons.blockedAgent, tokens.color.dim)
                                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    KText("Notifications", tokens.type.bodyStrong, tokens.color.text)
                                    KText(
                                        "Be told when an agent is blocked",
                                        tokens.type.captionSmall,
                                        tokens.color.dim,
                                    )
                                }
                                IconGlyph(KamprIcons.chevronRight, 13.dp, tokens.color.mute)
                            }
                        }
                    }
                }
                Box(Modifier.size(18.dp))
            }
            Box(Modifier.widthIn(max = 520.dp).padding(start = 18.dp, end = 18.dp, bottom = 18.dp)) {
                PrimaryAction("Open the herd", onOpenHerd, Modifier.fillMaxWidth())
            }
        }
        // Over everything, because a camera preview inside a scrolling column is a viewfinder
        // somebody has to keep in shot while they scroll.
        if (scanning) {
            // A window of its own, not a layer inside this screen: a viewfinder with the app's own
            // tab bar under it is a camera somebody is expected to navigate away from mid-aim, and
            // the system back gesture has to cancel the scan rather than leave the screen behind it.
            Dialog(
                onDismissRequest = { scanning = false },
                properties = DialogProperties(usePlatformDefaultWidth = false),
            ) {
                PairingScanSurface(
                    onScanned = { text ->
                        scanning = false
                        pairingFrom(text)?.let {
                            scanned = it
                            onConnect(it)
                        }
                    },
                    onClose = { scanning = false },
                )
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun RungCard(rung: Rung) {
    val tokens = Kampr.tokens
    val clickable = rung.onClick?.let { Modifier.touchable().action(rung.title, it) } ?: Modifier
    Surface(Modifier.fillMaxWidth().then(clickable)) {
        Row(
            Modifier.padding(horizontal = 15.dp, vertical = 13.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Badge(34.dp, 17.dp, rung.icon, if (rung.lead) tokens.color.accent else tokens.color.dim)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                KText(rung.title, tokens.type.bodyStrong, tokens.color.text)
                KText(rung.detail, tokens.type.captionSmall, tokens.color.dim, maxLines = 3)
                if (rung.unlocks.isNotEmpty()) {
                    FlowRow(
                        horizontalArrangement = Arrangement.spacedBy(5.dp),
                        verticalArrangement = Arrangement.spacedBy(5.dp),
                    ) {
                        for (chip in rung.unlocks) {
                            Pill(background = tokens.color.surface2, horizontal = 8.dp, vertical = 3.dp) {
                                KText(chip, tokens.type.micro, tokens.color.done)
                            }
                        }
                    }
                }
            }
            if (rung.onClick != null) {
                IconGlyph(KamprIcons.chevronRight, 13.dp, tokens.color.mute, Modifier.padding(top = 4.dp))
            }
        }
    }
}

@Composable
private fun MachineCard(node: NodeInfo) {
    val tokens = Kampr.tokens
    Surface(Modifier.fillMaxWidth()) {
        Row(
            Modifier.padding(horizontal = 15.dp, vertical = 13.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Mark(
                if (node.online) tokens.color.done else tokens.color.blocked,
                if (node.online) MarkShape.Bar else MarkShape.Ring,
                9.dp,
            )
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                KText(node.name, tokens.type.bodyStrong, tokens.color.text)
                // Version skew across a herd is invisible unless somebody prints it, and a herd
                // is exactly where two releases meet.
                KText(
                    listOfNotNull(
                        if (node.kind == "local") "this machine" else "peer",
                        node.build?.let { "kampr $it" },
                        node.herdrVersion?.let { "herdr $it" },
                        node.detail,
                    ).joinToString(" · "),
                    tokens.type.meta,
                    tokens.color.mute,
                    maxLines = 2,
                )
            }
        }
    }
}

@Composable
private fun PairingMark() {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    val cells = listOf(
        listOf(true, true, false, true),
        listOf(true, false, true, false),
        listOf(false, true, true, true),
        listOf(true, true, false, true),
    )
    Column(
        Modifier
            .size(44.dp)
            .named("Pairing code block")
            .background(tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .padding(6.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        for (row in cells) {
            Row(Modifier.weight(1f), horizontalArrangement = Arrangement.spacedBy(2.dp)) {
                for (on in row) {
                    Box(
                        Modifier
                            .weight(1f)
                            .fillMaxSize()
                            .background(if (on) tokens.color.text else tokens.color.raise)
                    )
                }
            }
        }
    }
}
