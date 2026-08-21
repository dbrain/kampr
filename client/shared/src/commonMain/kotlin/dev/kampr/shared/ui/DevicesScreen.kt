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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.DeviceRecord
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.util.relativeSeconds

@Composable
fun DevicesScreen(
    devices: List<DeviceRecord>,
    currentId: String?,
    now: Double,
    onBack: () -> Unit,
    onRevoke: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    // Revoking is the one action here that cannot be undone from a phone, and a mistap is a trip
    // to a shell on the host to pair again.
    var confirming by remember { mutableStateOf<String?>(null) }
    val live = devices.filter { it.active }
    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        Row(
            Modifier.fillMaxWidth().padding(start = 16.dp, top = 16.dp, end = 20.dp, bottom = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(11.dp),
        ) {
            BackAction("Back", onBack)
            KText("Devices", tokens.type.screenTitle, tokens.color.text, Modifier.weight(1f).asHeading())
        }
        Column(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).widthIn(max = 620.dp).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(9.dp),
        ) {
            if (live.isEmpty()) {
                KText("No devices reported by this node.", tokens.type.caption, tokens.color.mute)
            }
            for (device in live) {
                val current = device.id == currentId
                Surface(Modifier.fillMaxWidth()) {
                    Column(
                        Modifier.padding(horizontal = 15.dp, vertical = 13.dp),
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            Badge(
                                34.dp,
                                17.dp,
                                KamprIcons.lock,
                                if (current) tokens.color.accent else tokens.color.dim,
                                "Paired device",
                            )
                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                                KText(device.name, tokens.type.bodyStrong, tokens.color.text)
                                KText(
                                    listOfNotNull(
                                        device.role,
                                        device.lastSeenAt?.let { "seen ${relativeSeconds(it, now)}" },
                                        device.origin,
                                    ).joinToString(" · "),
                                    tokens.type.meta,
                                    tokens.color.mute,
                                    maxLines = 2,
                                )
                            }
                            if (current) {
                                StatusBadge(
                                    "this device", tokens.color.done, tokens.color.surface2,
                                    label = "This is the device you are using",
                                )
                            } else {
                                QuietAction(
                                    "Revoke", { confirming = device.id }, Modifier.widthIn(min = 92.dp),
                                    label = "Revoke ${device.name} — it loses access to this node",
                                )
                            }
                        }
                        if (confirming == device.id) {
                            Row(horizontalArrangement = Arrangement.spacedBy(9.dp)) {
                                KText(
                                    "It loses access within seconds, on any socket it already has open.",
                                    tokens.type.captionSmall,
                                    tokens.color.dim,
                                    Modifier.weight(1f),
                                    maxLines = 2,
                                )
                                QuietAction("Keep", { confirming = null }, Modifier.widthIn(min = 84.dp), label = "Keep ${device.name}")
                                PrimaryAction(
                                    "Revoke it",
                                    { confirming = null; onRevoke(device.id) },
                                    Modifier.widthIn(min = 96.dp),
                                    tokens.type.buttonSmall,
                                    10.dp,
                                    label = "Revoke ${device.name} now",
                                )
                            }
                        }
                    }
                }
            }
            Box(Modifier.padding(bottom = 18.dp))
        }
    }
}
