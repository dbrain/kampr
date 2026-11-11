package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
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
import dev.kampr.shared.theme.Kampr

val DIALOG_WIDTH = 480.dp

// The step a pane's menu or actions sheet nests into before it acts: a page of the same sheet, with
// a way back to the list and the commit at the bottom where every sheet keeps it.
@Composable
fun ColumnScope.RenamePage(
    kind: String,
    subject: String,
    name: String,
    clearable: Boolean,
    compact: Boolean,
    onBack: () -> Unit,
    onDismiss: () -> Unit,
    onRename: (String?) -> Unit,
) {
    val tokens = Kampr.tokens
    var label by remember(name) { mutableStateOf(TextFieldValue(name)) }
    val next = label.text.trim().ifEmpty { null }
    val allowed = next != null || clearable
    SheetHeader("Rename $kind", subject, onBack, onDismiss, compact)
    Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 6.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        KField(
            "name",
            label,
            style = tokens.type.body,
            label = "New name for this $kind",
            onSubmit = { if (allowed) onRename(next) },
        ) { label = it }
        if (clearable) KText("Leave it empty to go back to the name the $kind gives itself.", tokens.type.captionSmall, tokens.color.mute, maxLines = 2)
    }
    SheetActions("Rename", { onRename(next) }, onCancel = onBack, enabled = allowed, confirmLabel = "Save the new name for this $kind")
}

@Composable
fun ColumnScope.ConfirmPage(
    title: String,
    subject: String,
    detail: String?,
    confirm: String,
    compact: Boolean,
    onBack: () -> Unit,
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
    danger: Boolean = true,
) {
    val tokens = Kampr.tokens
    SheetHeader(title, null, onBack, onDismiss, compact, dismiss = "Close without changing anything")
    Column(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 20.dp, vertical = 4.dp)
            .announce(listOfNotNull(title, subject, detail, "Confirm or cancel.").joinToString(" "), urgent = true),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        KText(subject, tokens.type.bodyStrong, tokens.color.text, maxLines = 2)
        if (detail != null) KText(detail, tokens.type.captionSmall, tokens.color.mute, maxLines = 4)
    }
    SheetActions(confirm, onConfirm, onCancel = onBack, danger = danger)
}

@Composable
fun Refusal(text: String?) {
    val tokens = Kampr.tokens
    text ?: return
    KText(
        text,
        tokens.type.captionSmall,
        tokens.color.blocked,
        Modifier.padding(horizontal = 20.dp, vertical = 6.dp).announce(text, urgent = true),
        maxLines = 3,
    )
}
