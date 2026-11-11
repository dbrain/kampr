package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.PrimaryAction
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.edge
import dev.kampr.shared.wire.HooksScope
import dev.kampr.shared.wire.ServerMsg

private const val GIVES = "permission prompts, questions and replies arrive live instead of being read off the screen."
private const val EVERY_CLAUDE = "All projects writes ~/.claude/settings.json, for every Claude on this machine."

private fun scopeOf(wire: String?): HooksScope? = HooksScope.entries.firstOrNull { it.wire == wire }

private fun basename(path: String): String = path.trimEnd('/').substringAfterLast('/').ifEmpty { path }

// docs/12 W11, "Asking to install". A stale install is updated where it already is; one whose
// scope the node did not name is offered as if absent rather than guessed at.
@Composable
internal fun HooksOffer(
    offer: ServerMsg.ClaudeHooks,
    asking: Boolean,
    refusal: String?,
    onInstall: (HooksScope) -> Unit,
    onNotNow: () -> Unit,
    onDecline: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val update = scopeOf(offer.scope).takeIf { offer.state == "stale" }
    val shape = RoundedCornerShape(tokens.radii.md)
    Column(
        modifier
            .fillMaxWidth()
            .padding(top = 12.dp)
            .background(tokens.color.surface, shape)
            .edge(BorderSpec(1.dp, tokens.color.accent), shape)
            .padding(horizontal = 13.dp, vertical = 11.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        LabelText(if (update == null) "Kampr's Claude hooks" else "Kampr's Claude hooks are out of date", tokens.type.metaSmall, tokens.color.accent)
        val words = when (update) {
            null -> "Install them and $GIVES $EVERY_CLAUDE"
            HooksScope.User -> "Update them and $GIVES They are in ~/.claude/settings.json, for every Claude on this machine."
            HooksScope.Project -> "Update them and $GIVES"
        }
        KText(words, tokens.type.caption, tokens.color.dim, maxLines = 4)
        refusal?.let { KText(it, tokens.type.micro, tokens.color.blocked, maxLines = 3) }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            QuietAction("Not now", onNotNow, Modifier.weight(1f))
            QuietAction("Don't ask again", onDecline, Modifier.weight(1f), enabled = !asking)
        }
        if (update != null) {
            PrimaryAction("Update hooks", { onInstall(update) }, Modifier.fillMaxWidth(), tokens.type.buttonSmall, 10.dp, enabled = !asking)
        } else {
            offer.project?.let { project ->
                QuietAction("Install for ${basename(project)}", { onInstall(HooksScope.Project) }, Modifier.fillMaxWidth(), enabled = !asking)
            }
            PrimaryAction(
                "Install for all projects",
                { onInstall(HooksScope.User) },
                Modifier.fillMaxWidth(),
                tokens.type.buttonSmall,
                10.dp,
                enabled = !asking,
            )
        }
    }
}
