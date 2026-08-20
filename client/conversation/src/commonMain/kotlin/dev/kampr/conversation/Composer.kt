package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.TOUCH
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.edgeTop
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.readingOrder

// Herdr's send_text takes a JSON string, and a carriage return is what a harness reads as submit.
// It goes as its own message so a harness that debounces sees the text settle before the newline.
fun replyMessages(paneId: String, text: String): List<ClientMsg> =
    listOf(ClientMsg.InputText(paneId, text), ClientMsg.InputText(paneId, "\r"))

// A plain multi-line text box on purpose: the native keyboard's own dictation button then works,
// which no custom input surface can offer.
@Composable
fun Composer(agent: String?, enabled: Boolean, onSend: (String) -> Unit, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    var value by remember { mutableStateOf(TextFieldValue()) }
    val pill = RoundedCornerShape(tokens.radii.pill)
    val ready = enabled && value.text.isNotBlank()

    fun submit() {
        if (!ready) return
        onSend(value.text.trimEnd())
        value = TextFieldValue()
    }

    Row(
        modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeTop()
            .readingOrder(1f)
            .padding(start = 12.dp, top = 10.dp, end = 12.dp, bottom = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Box(
            Modifier
                .weight(1f)
                .background(tokens.color.surface, pill)
                .edge(tokens.card, pill)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            if (value.text.isEmpty()) {
                KText(
                    if (enabled) "Reply to ${agent ?: "the agent"}…" else "read-only device",
                    tokens.type.body,
                    tokens.color.mute,
                )
            }
            BasicTextField(
                value = value,
                onValueChange = { value = it },
                enabled = enabled,
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 96.dp)
                    .named(if (enabled) "Reply to ${agent ?: "the agent"}" else "Read-only device — replies are refused"),
                textStyle = tokens.type.body.copy(color = tokens.color.text),
                cursorBrush = SolidColor(tokens.color.accent),
            )
        }
        val pillShape = RoundedCornerShape(tokens.radii.pill)
        Box(
            Modifier
                .size(TOUCH)
                .background(if (ready) tokens.color.accent else tokens.color.raise, pillShape)
                .action(
                    "Send this reply to ${agent ?: "the agent"}",
                    { submit() },
                    pillShape,
                    enabled = ready,
                ),
            contentAlignment = Alignment.Center,
        ) {
            IconGlyph(ConversationIcons.send, 18.dp, if (ready) tokens.color.onAccent else tokens.color.mute)
        }
    }
}
