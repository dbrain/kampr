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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.DeviceRecord
import dev.kampr.shared.theme.Kampr

@Composable
fun DevicesScreen(
    devices: List<DeviceRecord>,
    onBack: () -> Unit,
    onRevoke: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
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
            if (devices.isEmpty()) {
                KText("No devices reported by this node.", tokens.type.caption, tokens.color.mute)
            }
            for (device in devices) {
                Surface(Modifier.fillMaxWidth()) {
                    Row(
                        Modifier.padding(horizontal = 15.dp, vertical = 13.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Badge(34.dp, 17.dp, KamprIcons.lock, if (device.current) tokens.color.accent else tokens.color.dim, "Paired device")
                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                            KText(device.name, tokens.type.bodyStrong, tokens.color.text)
                            KText(
                                listOfNotNull(device.kind, device.role, device.lastSeen).joinToString(" · "),
                                tokens.type.meta,
                                tokens.color.mute,
                            )
                        }
                        if (!device.current) {
                            QuietAction(
                                "Revoke", { onRevoke(device.id) }, Modifier.widthIn(min = 92.dp),
                                label = "Revoke ${device.name} — it loses access to this node",
                            )
                        } else {
                            StatusBadge(
                                "this device", tokens.color.done, tokens.color.surface2,
                                label = "This is the device you are using",
                            )
                        }
                    }
                }
            }
            Box(Modifier.padding(bottom = 18.dp))
        }
    }
}
