package dev.kampr.conversation

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KField
import dev.kampr.shared.ui.PrimaryAction
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.wire.PendingOption

@Composable
internal fun PendingWords(
    option: PendingOption,
    enabled: Boolean,
    onSend: (String) -> Unit,
    onCancel: () -> Unit,
) {
    var words by rememberSaveable(option.key, option.label) { mutableStateOf("") }
    val ready = enabled && words.isNotBlank()
    val send = { if (ready) onSend(words.trim()) }
    val focus = remember { FocusRequester() }
    Column(
        Modifier
            .fillMaxWidth()
            .onPreviewKeyEvent { event ->
                (event.type == KeyEventType.KeyDown && event.key == Key.Escape).also { if (it) onCancel() }
            },
        verticalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        KField(
            hint = option.label,
            text = words,
            modifier = Modifier.focusRequester(focus),
            label = "Your answer: ${option.label}",
            onSubmit = send,
        ) { words = it }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(7.dp)) {
            QuietAction("Cancel", onCancel, Modifier.weight(1f), label = "Put the answer away")
            PrimaryAction(
                "Send",
                send,
                Modifier.weight(1f),
                Kampr.tokens.type.buttonSmall,
                9.dp,
                enabled = ready,
                label = "Send your answer",
            )
        }
    }
    LaunchedEffect(Unit) { focus.requestFocus() }
}
