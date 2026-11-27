package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.StoredConversation
import kotlin.time.Duration.Companion.milliseconds

internal const val CLAUDE_CARD = "Claude (no terminal)"

internal val RESUMABLE_SETTLE = 300.milliseconds

// docs/21: `claude.create` with an optional first prompt, which the node sends once a pane opens
// the conversation; and the conversations on disk there, any of which `claude.resume` continues.
@Composable
internal fun ClaudeFields(
    cwd: TextFieldValue,
    onCwd: (TextFieldValue) -> Unit,
    prompt: TextFieldValue,
    onPrompt: (TextFieldValue) -> Unit,
    stored: List<StoredConversation>?,
    refusal: String?,
    nodeName: String,
    onResume: (String) -> Unit,
) {
    val tokens = Kampr.tokens
    Column(Modifier.padding(horizontal = 20.dp), verticalArrangement = Arrangement.spacedBy(11.dp)) {
        LabelledField("Directory", "/home/dbrain/dev/kampr", cwd, onChange = onCwd)
        LabelledField("First prompt", "optional", prompt, onChange = onPrompt)
        KText(
            "Claude on $nodeName with no terminal: a conversation whose questions are answered here. Its billing guards are checked before a word is sent.",
            tokens.type.captionSmall,
            tokens.color.mute,
            maxLines = 4,
        )
    }
    SheetSection("Resume…")
    Column(Modifier.padding(horizontal = 20.dp), verticalArrangement = Arrangement.spacedBy(9.dp)) {
        when {
            refusal != null -> KText(refusal, tokens.type.captionSmall, tokens.color.blocked, Modifier.announce(refusal), maxLines = 3)

            stored == null -> KText("Looking for conversations…", tokens.type.captionSmall, tokens.color.mute)

            stored.isEmpty() -> KText("No conversation to resume here.", tokens.type.captionSmall, tokens.color.mute)

            else -> for (conversation in stored) {
                SheetCard(
                    icon = KamprIcons.restore,
                    iconTint = tokens.color.dim,
                    title = conversation.title ?: conversation.id,
                    subtitle = conversation.cwd ?: conversation.id,
                    compact = true,
                    onClick = { onResume(conversation.id) },
                    label = "Resume ${conversation.title ?: conversation.id}",
                )
            }
        }
    }
}
