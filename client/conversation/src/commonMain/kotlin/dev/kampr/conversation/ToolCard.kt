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
import dev.kampr.shared.ui.Mark
import dev.kampr.shared.ui.MarkShape
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.wire.Block

const val TOOL_RUNNING = "running"
const val TOOL_ERROR = "error"

@Composable
fun ToolCard(
    tool: Block.Tool,
    detail: List<Block>,
    query: String,
    expanded: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val palette = rememberConversationPalette()
    val running = tool.state == TOOL_RUNNING
    val tone = when (tool.state) {
        TOOL_RUNNING -> tokens.color.working
        TOOL_ERROR -> tokens.color.blocked
        else -> tokens.color.dim
    }
    val outcome = when (tool.state) {
        TOOL_RUNNING -> "running"
        TOOL_ERROR -> "failed"
        else -> tool.lines?.let { "$it lines" } ?: "finished"
    }
    val named = listOfNotNull(tool.name, tool.summary).joinToString(", ")
    Surface(modifier.fillMaxWidth(), radius = tokens.radii.md) {
        Column {
            Row(
                Modifier
                    .fillMaxWidth()
                    .let {
                        if (detail.isEmpty()) it.named("Tool $named, $outcome")
                        else it.touchable().action(
                            if (expanded) "Hide the output of $named, $outcome"
                            else "Show the output of $named, $outcome",
                            onToggle,
                            selected = expanded,
                        )
                    }
                    .padding(horizontal = 12.dp, vertical = 9.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(9.dp),
            ) {
                Mark(
                    tone,
                    when (tool.state) {
                        TOOL_RUNNING -> MarkShape.Circle
                        TOOL_ERROR -> MarkShape.Square
                        else -> MarkShape.Bar
                    },
                    7.dp,
                )
                KText(
                    listOfNotNull(tool.name, tool.summary).joinToString(" · "),
                    tokens.type.meta,
                    if (running) tokens.color.text else tokens.color.dim,
                    Modifier.weight(1f),
                )
                when {
                    running -> KText("running", tokens.type.micro, tokens.color.working)
                    tool.state == TOOL_ERROR -> KText("failed", tokens.type.micro, tokens.color.blocked)
                    else -> tool.lines?.let { KText("$it lines", tokens.type.micro, tokens.color.mute) }
                }
                if (detail.isNotEmpty()) {
                    IconGlyph(
                        if (expanded) ConversationIcons.chevronUp else ConversationIcons.chevronDown,
                        12.dp,
                        tokens.color.mute,
                    )
                }
            }
            if (expanded && detail.isNotEmpty()) {
                Box(Modifier.fillMaxWidth().height(1.dp).background(palette.rule))
                Column(
                    Modifier.fillMaxWidth().padding(10.dp),
                    verticalArrangement = Arrangement.spacedBy(9.dp),
                ) {
                    for (block in detail) {
                        when (block) {
                            is Block.Code -> CodeCard(block.lang, block.text, query)
                            is Block.Diff -> DiffCard(block.path, block.text, query)
                            else -> Unit
                        }
                    }
                }
            }
        }
    }
}
