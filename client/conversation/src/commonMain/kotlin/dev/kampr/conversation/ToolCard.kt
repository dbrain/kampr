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
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.net.filePathOf
import dev.kampr.shared.wire.Block

const val TOOL_RUNNING = "running"
const val TOOL_ERROR = "error"

fun toolLabel(tool: Block.Tool, gap: String): String = tool.summary?.let { "${tool.name}$gap$it" } ?: tool.name

@Composable
fun ToolCard(
    tool: Block.Tool,
    detail: List<Block>,
    query: String,
    expanded: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
    sub: Block.Sub? = null,
    attachments: AttachmentStore = rememberAttachmentStore(""),
    output: Block.Code? = null,
) {
    val tokens = Kampr.tokens
    val palette = rememberConversationPalette()
    val running = tool.state == TOOL_RUNNING
    val tone = when (tool.state) {
        TOOL_RUNNING -> tokens.color.working
        TOOL_ERROR -> tokens.color.blocked
        else -> tokens.color.dim
    }
    // #173: a label that has to be guessed is a failed label. The count is the size of what the
    // tool wrote **back** — the node measures it off the result — and for a Bash call the body
    // under the chevron is one line of command, so a card that said "13 lines" and opened on one
    // was reporting two different things in the same breath.
    val outcome = when (tool.state) {
        TOOL_RUNNING -> "running"
        TOOL_ERROR -> "failed"
        else -> tool.lines?.let { "$it lines of output" } ?: "finished"
    }
    // What opening it shows, which is the result only where the node carried one.
    val opens = if (output != null) "the output of" else "what was sent to"
    val body = detail.isNotEmpty() || output != null
    val named = toolLabel(tool, ", ")
    // Lifted off the block it sits in rather than level with it. A call is a box inside the
    // reply's box, and `surface` is what that reply is already painted in — a card the same colour
    // as its own ground is not a card, it is a paragraph with a chevron on it.
    //
    // As wide as what is in it (see [`fitContent`]): a call reading four words does not need the
    // desktop column, and expanding one grows the card to whatever its output wants instead.
    Surface(modifier.fitContent(), background = tokens.color.raise, radius = tokens.radii.md) {
        Column {
            Row(
                Modifier
                    .fillMaxWidth()
                    .let {
                        if (!body) it.named("Tool $named, $outcome")
                        else it.touchable(LANDSCAPE_TOUCH).action(
                            if (expanded) "Hide $opens $named, $outcome"
                            else "Show $opens $named, $outcome",
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
                    toolLabel(tool, " · "),
                    tokens.type.meta,
                    if (running) tokens.color.text else tokens.color.dim,
                    Modifier.weight(1f),
                )
                when {
                    running -> KText("running", tokens.type.micro, tokens.color.working)
                    tool.state == TOOL_ERROR -> KText("failed", tokens.type.micro, tokens.color.blocked)
                    else -> tool.lines?.let {
                        KText("$it lines of output", tokens.type.micro, tokens.color.mute, maxLines = 2)
                    }
                }
                if (body) {
                    IconGlyph(
                        if (expanded) ConversationIcons.chevronUp else ConversationIcons.chevronDown,
                        12.dp,
                        tokens.color.mute,
                    )
                }
            }
            // What the call touched, under the call. The path is the node's own — `summary` is
            // filled from `file_path`/`path` for every read, edit and write — so this is a target
            // a reader cannot dispute rather than a guess at a path inside a sentence.
            val path = filePathOf(tool.summary)
            if (sub != null || path != null) {
                Box(Modifier.fillMaxWidth().height(1.dp).background(palette.rule))
                if (sub != null) SubCard(sub)
                if (path != null) FileAffordance(path, attachments)
            }
            if (expanded && body) {
                Box(Modifier.fillMaxWidth().height(1.dp).background(palette.rule))
                Column(
                    Modifier.fillMaxWidth().padding(10.dp),
                    verticalArrangement = Arrangement.spacedBy(9.dp),
                ) {
                    // Named only where there are two things to tell apart. On a card carrying the
                    // call and nothing else the header has already said so, and a label over the
                    // only thing in the box is chrome.
                    if (output != null && detail.isNotEmpty()) {
                        LabelText("sent", tokens.type.metaSmall, tokens.color.mute)
                    }
                    for (block in detail) {
                        when (block) {
                            is Block.Code -> CodeCard(block.lang, block.text, query)
                            is Block.Diff -> DiffCard(block.path, block.text, query, attachments = attachments)
                            else -> Unit
                        }
                    }
                    output?.let { ToolOutput(it, tool.lines, query, labelled = detail.isNotEmpty()) }
                }
            }
        }
    }
}

// The tool's own result, and how much of it this is. The node caps what it carries and leaves the
// `tool` block's `lines` as the true total, so a body shorter than the count is a body that was
// cut — said the way the file viewer already says it rather than in a second wording.
@Composable
private fun ToolOutput(block: Block.Code, total: Int?, query: String, labelled: Boolean) {
    val tokens = Kampr.tokens
    if (labelled) LabelText("output", tokens.type.metaSmall, tokens.color.mute)
    CodeCard(block.lang, block.text, query)
    val shown = linesOf(block.text)
    if (total != null && total > shown) {
        KText(
            "showing the first $shown of $total lines",
            tokens.type.micro,
            tokens.color.mute,
            Modifier.announce("Showing the first $shown of $total lines"),
            maxLines = 2,
        )
    }
}

internal fun linesOf(text: String): Int =
    if (text.isEmpty()) 0 else text.trimEnd('\n').count { it == '\n' } + 1
