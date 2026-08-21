package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.endpointFrom
import dev.kampr.shared.theme.Kampr

// Without this the app can only reach whatever `defaultEndpoint()` derived, which on an installed
// app is nothing at all — no real phone can pair.
@Composable
fun ConnectPanel(
    current: Endpoint?,
    error: String?,
    onConnect: (Endpoint) -> Unit,
    onPasskey: ((Endpoint) -> Unit)? = null,
    recent: List<String> = emptyList(),
    offeredCode: String? = null,
    onScan: (() -> Unit)? = null,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    var address by remember(current?.baseUrl) { mutableStateOf(TextFieldValue(current?.baseUrl.orEmpty())) }
    // Deliberately empty rather than pre-filled with the stored token: a blank code means "keep
    // the enrolment I already have", which is what editing the address alone must not throw away.
    // A code carried in on a scanned link is the one exception — it is what the scan was for.
    var code by remember(offeredCode) { mutableStateOf(TextFieldValue(offeredCode.orEmpty())) }

    val target = endpointFrom(address.text, code.text)

    Surface(modifier.fillMaxWidth()) {
        Column(
            Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            LabelText("Point Kampr at a node", tokens.type.captionSmall, tokens.color.mute)
            KText(
                "The address the node printed when you ran `kampr init` — the machine's own name " +
                    "or LAN address, and port 8790 unless you changed it.",
                tokens.type.captionSmall,
                tokens.color.dim,
                maxLines = 3,
            )
            KField("192.168.1.24:8790", address, label = "Node address") { address = it }
            // The scheme is inferred from the shape of the host, so it is printed back rather
            // than applied silently: a wrong guess has to be one the operator can see.
            if (target != null && target.baseUrl != address.text.trim()) {
                KText("connects to ${target.baseUrl}", tokens.type.meta, tokens.color.mute)
            }
            RecentAddresses(recent) { address = TextFieldValue(it, TextRange(it.length)) }
            KField("pairing code", code, label = "Pairing code, only when pairing") { code = it }
            if (error != null) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .background(tokens.color.blockedBg, RoundedCornerShape(tokens.radii.sm))
                        .announce(error, urgent = true)
                        .padding(horizontal = 11.dp, vertical = 9.dp),
                    horizontalArrangement = Arrangement.spacedBy(9.dp),
                ) {
                    IconGlyph(KamprIcons.warning, 14.dp, tokens.color.blocked)
                    KText(error, tokens.type.captionSmall, tokens.color.text, maxLines = 4)
                }
            }
            // The camera is offered above the buttons rather than instead of them: a phone whose
            // owner refuses the camera has to be able to finish here, by typing.
            if (onScan != null) {
                QuietAction(
                    "Scan a pairing code",
                    onScan,
                    Modifier.fillMaxWidth(),
                    vertical = 12.dp,
                    label = "Scan a pairing code with the camera",
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(9.dp)) {
                PrimaryAction(
                    "Connect",
                    { target?.let(onConnect) },
                    Modifier.weight(1f),
                    tokens.type.buttonSmall,
                    12.dp,
                    enabled = target != null,
                    label = "Connect to this node",
                )
                // The only enrolment path that does not need somebody at the console: a pairing
                // code printed there is inert until it is armed there.
                if (onPasskey != null) {
                    QuietAction(
                        "Passkey",
                        { target?.let { onPasskey(it.copy(token = null)) } },
                        Modifier.weight(1f),
                        vertical = 12.dp,
                        enabled = target != null,
                        label = "Sign in to this node with a passkey",
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun RecentAddresses(recent: List<String>, onPick: (String) -> Unit) {
    if (recent.isEmpty()) return
    val tokens = Kampr.tokens
    Column(verticalArrangement = Arrangement.spacedBy(5.dp)) {
        LabelText("used before", tokens.type.micro, tokens.color.mute)
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            for (url in recent) {
                Chip(url, false, { onPick(url) }, quiet = true, label = "Use $url")
            }
        }
    }
}
