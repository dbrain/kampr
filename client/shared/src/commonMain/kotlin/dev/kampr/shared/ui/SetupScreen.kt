package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.SetupStatus
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.Security

private data class Rung(
    val icon: Icon,
    val title: String,
    val detail: String,
    val unlocks: List<String> = emptyList(),
    val lead: Boolean = false,
)

private val unlockCopy = mapOf(
    "passkeys" to "unlocks passkeys",
    "push" to "notifications",
    "installable" to "install to home screen",
)

// The ladder is built from hello.security, never from the URL: a rung whose affordance cannot
// work on this node is absent rather than present-and-failing.
private fun ladderFor(security: Security): List<Rung> = buildList {
    if (security.passkeys) {
        add(
            Rung(
                KamprIcons.lock,
                "Add a passkey",
                "This node is reachable at a name with a certificate, so WebAuthn works here.",
                lead = true,
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
    endpoint: Endpoint,
    onConnect: (Endpoint) -> Unit,
    onOpenHerd: () -> Unit,
    onDevices: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            Column(
                Modifier.widthIn(max = 520.dp).padding(start = 22.dp, top = 22.dp, end = 22.dp),
                verticalArrangement = Arrangement.spacedBy(9.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(9.dp)) {
                    Dot(if (running) tokens.color.done else tokens.color.blocked, 9.dp)
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
                    maxLines = 2,
                )
                KText(
                    "Nothing to configure. Everything below is optional, and each one says what it buys you.",
                    tokens.type.body,
                    tokens.color.dim,
                    maxLines = 3,
                )
            }

            if (!running) {
                Box(Modifier.widthIn(max = 520.dp).padding(start = 18.dp, top = 16.dp, end = 18.dp)) {
                    ConnectPanel(endpoint, onConnect)
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
                                    status?.address ?: "not paired",
                                    tokens.type.meta.copy(fontSize = tokens.type.cardTitle.fontSize),
                                    tokens.color.text,
                                )
                                KText(
                                    status?.pairingCode?.let { "pair with $it" } ?: "no pairing code offered",
                                    tokens.type.captionSmall,
                                    tokens.color.mute,
                                )
                            }
                        }
                        if (security.unencryptedBanner) {
                            val shape = RoundedCornerShape(tokens.radii.sm)
                            Row(
                                Modifier
                                    .fillMaxWidth()
                                    .background(tokens.color.blockedBg, shape)
                                    .border(1.dp, tokens.color.blocked, shape)
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
                for (rung in ladderFor(security)) RungCard(rung)
            }

            Box(Modifier.padding(start = 22.dp, top = 20.dp, end = 22.dp, bottom = 9.dp)) {
                LabelText("This device", tokens.type.captionSmall, tokens.color.mute)
            }
            Box(Modifier.widthIn(max = 520.dp).padding(horizontal = 18.dp, vertical = 0.dp)) {
                Surface(Modifier.fillMaxWidth().clickable(onClick = onDevices)) {
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
            Box(Modifier.size(18.dp))
        }
        Box(Modifier.widthIn(max = 520.dp).padding(start = 18.dp, end = 18.dp, bottom = 18.dp)) {
            PrimaryAction("Open the herd", onOpenHerd, Modifier.fillMaxWidth())
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun RungCard(rung: Rung) {
    val tokens = Kampr.tokens
    Surface(Modifier.fillMaxWidth()) {
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
            IconGlyph(KamprIcons.chevronRight, 13.dp, tokens.color.mute, Modifier.padding(top = 4.dp))
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
        Modifier.size(44.dp).background(tokens.color.raise, shape).edge(tokens.card, shape).padding(6.dp),
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
