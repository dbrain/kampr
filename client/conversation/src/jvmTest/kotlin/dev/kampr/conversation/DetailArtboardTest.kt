package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import java.io.File
import kotlin.test.Test

private val OUT = File("build/artboards")

class DetailArtboardTest {
    private fun revisedEditTurn() =
        (Wire.decode(RICH_REVISION) as ServerMsg.ConvoTurn).turns.first { it.blocks.any { b -> b is Block.Diff } }

    // The collapsed state is what every other artboard shows; this is the other half of it, plus
    // the diff and search highlighting that only appear once a reader goes looking.
    @Test
    fun expandedToolDiffAndSearchHighlightRender() {
        val turn = revisedEditTurn()
        val call = groupBlocks(turn.blocks).single() as Piece.Call
        renderArtboard(390.dp, 760.dp, SoftTheme, TypeScale.Phone, File(OUT, "detail-390.png")) {
            val tokens = Kampr.tokens
            Column(
                Modifier.fillMaxSize().background(tokens.color.bg).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                ToolCard(call.tool, call.detail, query = "", expanded = true, onToggle = {})
                ToolCard(
                    Block.Tool("Bash", "cargo test -p kampr-journal", null, TOOL_RUNNING),
                    detail = emptyList(),
                    query = "",
                    expanded = false,
                    onToggle = {},
                )
                ToolCard(
                    Block.Tool("Bash", "cargo clippy --all-targets", 12, TOOL_ERROR),
                    detail = emptyList(),
                    query = "",
                    expanded = false,
                    onToggle = {},
                )
                dev.kampr.conversation.md.Markdown(
                    "A short read can also be **truncated**, which is the whole of probe #55.",
                    query = "truncated",
                )
                CodeCard("bash", "herdr pane list --json | jq '.panes[] | .id'", query = "jq")
            }
        }
    }
}
