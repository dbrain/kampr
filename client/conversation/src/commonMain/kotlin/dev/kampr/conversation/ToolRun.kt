package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.Mark
import dev.kampr.shared.ui.MarkShape
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.touchable

// Three, not two: a row reading "2 tool calls" costs a tap and saves one line of screen, and the
// reader who tapped it has to tap again to get back what they could already see.
const val TOOL_RUN_MIN = 3

// The collapsed row, and the first of the two taps: this one hands back the individual cards, and
// each of those still opens its own output. A run that hid a failure or a call still in flight
// would be a broken thing wearing a healthy face (#233), so both are on the row and in its label
// before anyone taps anything.
@Composable
fun ToolRunCard(
    row: TranscriptRow.Run,
    expanded: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
    calls: @Composable () -> Unit,
) {
    val tokens = Kampr.tokens
    val palette = rememberConversationPalette()
    val tools = row.tools
    val failed = tools.count { it.state == TOOL_ERROR }
    val running = tools.count { it.state == TOOL_RUNNING }
    val names = tools.map { it.name }.distinct()
    val tone = when {
        failed > 0 -> tokens.color.blocked
        running > 0 -> tokens.color.working
        else -> tokens.color.dim
    }
    val outcome = listOfNotNull(
        if (failed > 0) "$failed failed" else null,
        if (running > 0) "$running running" else null,
    ).joinToString(", ")
    val held = "${tools.size} tool calls, ${names.joinToString(", ")}" +
        if (outcome.isEmpty()) "" else ", $outcome"
    Surface(modifier.fitContent(), background = tokens.color.raise, radius = tokens.radii.md) {
        Column {
            Row(
                Modifier
                    .fillMaxWidth()
                    .touchable(LANDSCAPE_TOUCH)
                    .action(if (expanded) "Hide $held" else "Show $held", onToggle, selected = expanded)
                    .padding(horizontal = 12.dp, vertical = 9.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(9.dp),
            ) {
                Mark(
                    tone,
                    when {
                        failed > 0 -> MarkShape.Square
                        running > 0 -> MarkShape.Circle
                        else -> MarkShape.Bar
                    },
                    7.dp,
                )
                KText("${tools.size} tool calls", tokens.type.meta, tokens.color.text)
                KText(names.joinToString(" · "), tokens.type.meta, tokens.color.dim, Modifier.weight(1f))
                if (outcome.isNotEmpty()) {
                    KText(
                        outcome,
                        tokens.type.micro,
                        if (failed > 0) tokens.color.blocked else tokens.color.working,
                    )
                }
                IconGlyph(
                    if (expanded) ConversationIcons.chevronUp else ConversationIcons.chevronDown,
                    12.dp,
                    tokens.color.mute,
                )
            }
            if (expanded) {
                Box(Modifier.fillMaxWidth().height(1.dp).background(palette.rule))
                Column(
                    Modifier.fillMaxWidth().padding(10.dp),
                    verticalArrangement = Arrangement.spacedBy(9.dp),
                ) { calls() }
            }
        }
    }
}
