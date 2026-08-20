package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.theme.Kampr

// Without this the app can only reach whatever `defaultEndpoint()` guessed, which is a
// loopback address on desktop and an emulator alias on Android — no real phone can pair.
@Composable
fun ConnectPanel(current: Endpoint, onConnect: (Endpoint) -> Unit, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    var address by remember(current.baseUrl) { mutableStateOf(TextFieldValue(current.baseUrl)) }
    var code by remember { mutableStateOf(TextFieldValue(current.token.orEmpty())) }

    Surface(modifier.fillMaxWidth()) {
        Column(
            Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            LabelText("Point Kampr at a node", tokens.type.captionSmall, tokens.color.mute)
            KField("http://192.168.1.24:8790", address, label = "Node address") { address = it }
            KField("pairing code", code, label = "Pairing code") { code = it }
            Row(horizontalArrangement = Arrangement.spacedBy(9.dp)) {
                PrimaryAction(
                    "Connect",
                    {
                        val host = address.text.trim().ifEmpty { current.baseUrl }
                        val url = if (host.startsWith("http")) host else "http://$host"
                        onConnect(Endpoint(url, code.text.trim().ifEmpty { null }))
                    },
                    Modifier.weight(1f),
                    tokens.type.buttonSmall,
                    12.dp,
                    label = "Connect to this node",
                )
            }
        }
    }
}
