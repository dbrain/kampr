package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.md.Markdown
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.announce
import dev.kampr.shared.wire.Attachment
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn

/// The node's reserved id for the message a harness is still writing. It is scraped off the pane's
// screen, so it is an approximation of a turn that does not exist yet: it is revised as the text
// grows, and withdrawn — same id, no blocks — the moment the harness writes the real record.
const val LIVE_TURN_ID = "live"

/** A withdrawn live turn carries no blocks and is not a turn any more. */
fun Turn.isVisible(): Boolean = blocks.isNotEmpty()

sealed interface Piece {
    data class Prose(val text: String) : Piece
    data class Fence(val lang: String?, val text: String) : Piece
    data class Patch(val path: String?, val text: String) : Piece
    data class Call(val tool: Block.Tool, val detail: List<Block>) : Piece
    data class Attach(val att: Attachment) : Piece
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
            is Block.Md -> {
                // The marker the node writes beside a header — `[image · png]` — is a fallback for
                // a client that cannot fetch, and this one can: the card says the same thing and
                // presses.
                out += block.att?.let { Piece.Attach(it) } ?: Piece.Prose(block.text)
                index++
            }
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
    attachments: AttachmentStore = rememberAttachmentStore(""),
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
                        else PieceView(turn.id, index, piece, query, expanded, onToggle, attachments)
                    }
                }
            }
        }
        return
    }
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(11.dp)) {
        for ((index, piece) in pieces.withIndex()) {
            PieceView(turn.id, index, piece, query, expanded, onToggle, attachments)
        }
        if (turn.id == LIVE_TURN_ID) StreamingStrip()
    }
}

// Says out loud that this text came off the screen rather than out of the transcript. It is the
// only part of a turn a reader can act on differently: the wording may still change, and it will
// be replaced wholesale when the harness records it.
@Composable
private fun StreamingStrip() {
    val tokens = Kampr.tokens
    Row(
        Modifier.fillMaxWidth().announce("still writing"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        Box(
            Modifier.size(width = 7.dp, height = 13.dp)
                .background(tokens.color.accent, RoundedCornerShape(tokens.radii.sm)),
        )
        KText("still writing", tokens.type.meta, tokens.color.mute)
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
    attachments: AttachmentStore,
) {
    when (piece) {
        is Piece.Prose -> Markdown(piece.text, query, Modifier.fillMaxWidth())
        is Piece.Attach -> AttachmentCard(piece.att, attachments, Modifier.fillMaxWidth())
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
