package dev.kampr.conversation

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.md.Markdown
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn

sealed interface Piece {
    data class Prose(val text: String) : Piece
    data class Fence(val lang: String?, val text: String) : Piece
    data class Patch(val path: String?, val text: String) : Piece
    data class Call(val tool: Block.Tool, val detail: List<Block>) : Piece
}

// A tool call and the code or patch that follows it in the same turn are one thing to a reader,
// so the call owns them and collapsing hides them together.
fun groupBlocks(blocks: List<Block>): List<Piece> {
    val out = mutableListOf<Piece>()
    var index = 0
    while (index < blocks.size) {
        when (val block = blocks[index]) {
            is Block.Tool -> {
                val detail = mutableListOf<Block>()
                index++
                while (index < blocks.size && blocks[index].let { it is Block.Code || it is Block.Diff }) {
                    detail += blocks[index]
                    index++
                }
                out += Piece.Call(block, detail)
            }
            is Block.Md -> { out += Piece.Prose(block.text); index++ }
            is Block.Code -> { out += Piece.Fence(block.lang, block.text); index++ }
            is Block.Diff -> { out += Piece.Patch(block.path, block.text); index++ }
            is Block.Unknown -> index++
        }
    }
    return out
}

@Composable
fun TurnView(
    turn: Turn,
    query: String,
    expanded: List<String>,
    onToggle: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val pieces = groupBlocks(turn.blocks)
    if (turn.role == "user") {
        // A reply is not a document: it hugs its own text and leaves a gutter, so the eye can
        // tell who spoke without reading a word of it.
        Box(modifier.fillMaxWidth().padding(start = 44.dp), contentAlignment = Alignment.CenterEnd) {
            Surface(
                Modifier.widthIn(max = 460.dp),
                background = tokens.color.raise,
                radius = tokens.radii.lg,
            ) {
                Column(
                    Modifier.padding(horizontal = 14.dp, vertical = 11.dp),
                    verticalArrangement = Arrangement.spacedBy(9.dp),
                ) {
                    for ((index, piece) in pieces.withIndex()) {
                        if (piece is Piece.Prose) Markdown(piece.text, query)
                        else PieceView(turn.id, index, piece, query, expanded, onToggle)
                    }
                }
            }
        }
        return
    }
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(11.dp)) {
        for ((index, piece) in pieces.withIndex()) {
            PieceView(turn.id, index, piece, query, expanded, onToggle)
        }
    }
}

@Composable
private fun PieceView(
    turnId: String,
    index: Int,
    piece: Piece,
    query: String,
    expanded: List<String>,
    onToggle: (String) -> Unit,
) {
    when (piece) {
        is Piece.Prose -> Markdown(piece.text, query, Modifier.fillMaxWidth())
        is Piece.Fence -> CodeCard(piece.lang, piece.text, query)
        is Piece.Patch -> DiffCard(piece.path, piece.text, query)
        is Piece.Call -> {
            val key = "$turnId#$index"
            ToolCard(
                tool = piece.tool,
                detail = piece.detail,
                query = query,
                expanded = key in expanded,
                onToggle = { onToggle(key) },
            )
        }
    }
}
