package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr

// The node answers `native.create` without herdr, so a machine whose herdr is down can still be
// given one. Only the desk half asks it for a pane.
@Composable
internal fun NativeSessionFields(
    command: TextFieldValue,
    onCommand: (TextFieldValue) -> Unit,
    cwd: TextFieldValue,
    onCwd: (TextFieldValue) -> Unit,
    label: TextFieldValue,
    onLabel: (TextFieldValue) -> Unit,
    desk: Boolean?,
    onDesk: (Boolean) -> Unit,
    beside: String?,
) {
    val tokens = Kampr.tokens
    Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(11.dp)) {
        LabelledField("command", "the login shell", command, onChange = onCommand)
        LabelledField("directory", "/home/dbrain/dev/kampr", cwd, onChange = onCwd)
        LabelledField("label", "build", label, onChange = onLabel)
        if (desk != null) {
            Toggle(
                desk,
                "Also show on the desk",
                beside?.let { "in a herdr pane beside $it" } ?: "in a herdr pane on that machine",
                onDesk,
            )
        }
        KText(
            "Kept by Kampr rather than herdr, so it survives herdr restarting and lives on the desk only while something shows it there.",
            tokens.type.captionSmall,
            tokens.color.mute,
            maxLines = 4,
        )
    }
}
