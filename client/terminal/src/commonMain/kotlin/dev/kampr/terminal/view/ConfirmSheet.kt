package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.PrimaryAction
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.escapes
import dev.kampr.shared.ui.gestureAction
import dev.kampr.shared.ui.readingOrder
import dev.kampr.shared.ui.touchable
import dev.kampr.terminal.guard.HeldSubmit

// Never a bare "are you sure": the command that tripped the guard is on the sheet, verbatim, so
// the answer is a reading rather than a guess.
@Composable
fun ConfirmSheet(
    held: HeldSubmit,
    onRun: () -> Unit,
    onEdit: () -> Unit,
    onMute: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val heading = if (held.paste) "Pasted command" else "Before this runs"
    Box(Modifier.fillMaxSize().gestureAction("Back to edit", onEdit))
    Column(
        modifier
            .fillMaxWidth()
            .escapes(onEdit)
            .readingOrder(-1f)
            .announce(
                "$heading. ${held.reason}. The command is: ${held.command}",
                urgent = true,
            )
            .padding(horizontal = 12.dp, vertical = 11.dp),
    ) {
        Surface(Modifier.fillMaxWidth(), background = tokens.color.surface, radius = tokens.radii.lg) {
            Column(
                Modifier.padding(horizontal = 15.dp, vertical = 14.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    IconGlyph(KamprIcons.warning, 15.dp, tokens.color.blocked)
                    LabelText(
                        heading,
                        tokens.type.sectionLabel,
                        tokens.color.blocked,
                        Modifier.weight(1f),
                    )
                    KText(if (held.paste) "paste" else "enter", tokens.type.metaSmall, tokens.color.mute)
                }

                KText(held.reason, tokens.type.body, tokens.color.text, maxLines = 3)

                Surface(
                    Modifier.fillMaxWidth(),
                    background = tokens.color.surface2,
                    radius = tokens.radii.md,
                ) {
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .horizontalScroll(rememberScrollState())
                            .padding(horizontal = 12.dp, vertical = 11.dp),
                    ) {
                        KText(held.command, tokens.type.meta, tokens.color.text, maxLines = 4)
                    }
                }

                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    QuietAction(
                        "Back to edit", onEdit, Modifier.weight(1f), vertical = 13.dp,
                        label = "Back to edit — do not run it",
                    )
                    PrimaryAction(
                        "Run it", onRun, Modifier.weight(1f), vertical = 13.dp,
                        label = "Run ${held.command}",
                    )
                }

                val shape = RoundedCornerShape(tokens.radii.md)
                Row(
                    Modifier
                        .fillMaxWidth()
                        .edge(tokens.card, shape)
                        .touchable()
                        .action("Run it, and stop checking destructive commands in this pane", onMute, shape)
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    KText("Run it, and stop asking in this pane", tokens.type.buttonSmall, tokens.color.dim, Modifier.weight(1f))
                    IconGlyph(KamprIcons.chevronRight, 11.dp, tokens.color.mute)
                }

                KText(
                    "A mistap guard, not a lock — anyone who can reach this pane can already run " +
                        "anything. Turn it back on from the zoom sheet.",
                    tokens.type.captionSmall,
                    tokens.color.mute,
                    maxLines = 2,
                )
            }
        }
    }
}
