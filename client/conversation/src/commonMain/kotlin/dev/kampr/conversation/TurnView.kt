package dev.kampr.conversation

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.text.selection.DisableSelection
import dev.kampr.conversation.md.Breaks
import dev.kampr.conversation.md.Markdown
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.net.filePathOf
import dev.kampr.shared.net.wallClockMillis
import dev.kampr.shared.wire.Attachment
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn

// How much of a header a turn carries. An ask is a block of its own and wears the whole thing; a
// step inside a reply wears its clock, because the reply's head cannot say when each of thirty
// calls happened; a call inside a collapsed run wears nothing, the run's card being the header.
sealed interface TurnHead {
    data object Full : TurnHead
    data class Stamp(val text: String?) : TurnHead
    data object None : TurnHead
}

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
    data class Call(
        val tool: Block.Tool,
        val detail: List<Block>,
        val sub: Block.Sub? = null,
        val output: Block.Code? = null,
    ) : Piece
    data class Attach(val att: Attachment) : Piece
    // A picture the operator handed over, shown where they named it. The node writes the bytes on
    // the pane's own machine and types the path in, so the transcript carries a path string and
    // nothing else says a picture was ever handed over.
    data class Picture(val att: Attachment) : Piece
    // A recording a message named, offered where it named it. A row and not a fetch, which is what
    // makes it safe in an agent's reply as well as in the operator's own.
    data class Sound(val path: String) : Piece
    // A launched conversation with no call in front of it. The node writes the two together, so
    // this is the shape a page that opened between them arrives in rather than the ordinary one.
    data class Launch(val sub: Block.Sub) : Piece
}

// The wire's word for a tool's own result: a `code` block carrying `role: "output"`. It is the
// *last* block of the call's own run — after the input, after any launch and any patch, and before
// the next `tool` block — because a record can carry several calls and appending an answer to the
// turn would file the first call's under the second. The run below is already walked exactly that
// far, so the result arrives with the call it belongs to for free.
//
// The `tool` block's `lines` stays the **true** total against a body the node caps at 8 KiB or 120
// lines, so a body shorter than the count is one that was cut and the card says so. Most cards
// carry none: the node writes one for `Bash`, `Glob` and `Grep`, and for any tool that failed.
const val TOOL_OUTPUT = "output"

private fun isToolOutput(block: Block.Code): Boolean = block.role == TOOL_OUTPUT

// How many pictures one paragraph may put on the screen without being asked for. Each is an
// authorised fetch and, decoded, several megabytes of pixels — [AttachmentStore] holds four across
// the whole pane — so two is the most one block can ask for and still leave room for the message
// before it.
private const val MOST_PICTURES_INLINE = 2

// How many recordings one paragraph may put a row under. A row is a name and a press — no fetch,
// no decode, nothing held — so the ceiling is about a reply that names a directory's worth of
// files rather than about what the device can carry, and an agent that made eight clips should
// not have five of them silently dropped.
private const val MOST_SOUNDS_INLINE = 8

// The paths a message names, split the one way both scans below agree on.
private fun pathsIn(text: String): List<String> = text
    .split(' ', '\t', '\n', '\r')
    .mapNotNull { filePathOf(it.trim('`', '"', '\'', '(', ')', ',', ';', ':')) }
    .distinct()

// The pictures the operator's own message names, to be shown where it named them.
//
// Deliberately narrow, for the reason `filePathOf` refuses to search prose at all: a token is
// offered only where it is an absolute or `~/`-anchored path **and** ends in one of the extensions
// the node will serve inline. `/etc` in a sentence is not a picture; `/tmp/kampr-3f2.png` is.
fun picturesIn(text: String): List<Attachment> = pathsIn(text)
    .map(::fileTarget)
    .filter { offerFor(it) == AttachmentOffer.Image }
    .take(MOST_PICTURES_INLINE)

// The recordings a message names, offered where it named them.
//
// The same narrow rule the pictures follow, and offered in an agent's reply as well as in the
// operator's own — which a picture is not. A picture is *fetched* on sight, so forty in a reply is
// forty authorised round trips and several hundred megabytes of pixels; this is a line of text
// until somebody presses it. An agent that produced a `.wav` and typed its path is the only way a
// recording reaches a transcript at all: nothing writes it as an attachment, and a `Bash` call's
// summary is its `description` rather than its output path.
fun soundsIn(text: String): List<String> = pathsIn(text)
    .filter { soundType(it) != null }
    .take(MOST_SOUNDS_INLINE)

// A tool call and the code or patch that follows it in the same turn are one thing to a reader,
// so the call owns them and collapsing hides them together.
fun groupBlocks(blocks: List<Block>, pictures: Boolean = false): List<Piece> {
    val out = mutableListOf<Piece>()
    var index = 0
    while (index < blocks.size) {
        when (val block = blocks[index]) {
            is Block.Tool -> {
                val detail = mutableListOf<Block>()
                var launched: Block.Sub? = null
                var result: Block.Code? = null
                index++
                // The launch rides between the card and its output, so both are collected here:
                // what an agent was told to do and what it wrote back are one thing to a reader,
                // and collapsing the call has to take them together.
                while (index < blocks.size) {
                    when (val next = blocks[index]) {
                        is Block.Code -> if (isToolOutput(next)) result = result ?: next else detail += next
                        is Block.Diff -> detail += next
                        is Block.Sub -> launched = launched ?: next
                        else -> break
                    }
                    index++
                }
                out += Piece.Call(block, detail, launched, result)
            }
            is Block.Md -> {
                // The marker the node writes beside a header — `[image · png]` — is a fallback for
                // a client that cannot fetch, and this one can: the card says the same thing and
                // presses.
                val att = block.att
                if (att != null) {
                    out += Piece.Attach(att)
                } else {
                    // The prose stays whatever it was. A picture is added *beside* the path rather
                    // than in place of it, which is what makes a fetch that fails degrade to what
                    // is on the screen today instead of to an empty frame (#233).
                    out += Piece.Prose(block.text)
                    if (pictures) for (shot in picturesIn(block.text)) out += Piece.Picture(shot)
                    for (heard in soundsIn(block.text)) out += Piece.Sound(heard)
                }
                index++
            }
            is Block.Code -> { out += Piece.Fence(block.lang, block.text); index++ }
            is Block.Diff -> { out += Piece.Patch(block.path, block.text); index++ }
            is Block.Sub -> { out += Piece.Launch(block); index++ }
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
    now: Double = wallClockMillis(),
    agent: String? = null,
    framed: Boolean = true,
    head: TurnHead = TurnHead.Full,
    edge: BlockEdge = BlockEdge.Only,
) {
    val hand = typedByHand(turn)
    val pieces = groupBlocks(turn.blocks, pictures = hand)
    // A folded turn is not composed rather than not painted, which is what keeps its text out of
    // a selection drag as well as off the screen. It unfolds itself around a search match for the
    // reason a run of tool calls does: a hit the counter promises and the screen hides is worse
    // than a screen that is too long.
    val fold = foldKey(turn)
    val folded = turnFolded(turn, expanded) && !turnMatches(turn, query)
    val skin = speakerSkin(speakerOf(turn), agent)

    // Nested inside an expanded run, the run's own card is already the frame — a second one around
    // every call in it is a box drawn inside a box.
    if (!framed) {
        Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (head is TurnHead.Stamp) StepStamp(head.text)
            Body(turn, pieces, query, expanded, onToggle, attachments, hand)
        }
        return
    }

    BlockFrame(skin, edge, modifier) {
        when (head) {
            is TurnHead.Full -> TurnHeader(
                skin = skin,
                stamp = turnStamp(turn.at, now),
                gist = turnGist(turn),
                parts = pieces.size,
                folded = folded,
                onToggle = fold?.let { key -> { onToggle(key) } },
            )
            is TurnHead.Stamp -> StepStamp(head.text)
            TurnHead.None -> Unit
        }
        if (head !is TurnHead.Full || !folded) Body(turn, pieces, query, expanded, onToggle, attachments, hand)
    }
}

@Composable
private fun Body(
    turn: Turn,
    pieces: List<Piece>,
    query: String,
    expanded: List<String>,
    onToggle: (String) -> Unit,
    attachments: AttachmentStore,
    hand: Boolean,
) {
    for ((index, piece) in pieces.withIndex()) {
        PieceView(turn.id, index, piece, query, expanded, onToggle, attachments, hand)
    }
}

// A step's own clock, and nothing else. Who is speaking is the reply's business and it says so
// once at the top; when *this* step happened is not something the head can answer for it, and a
// reader working out where an agent spent eleven minutes needs every one of them.
@Composable
fun StepStamp(stamp: String?) {
    if (stamp == null) return
    val tokens = Kampr.tokens
    DisableSelection { KText(stamp, tokens.type.micro, tokens.color.mute) }
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
    hand: Boolean,
) {
    when (piece) {
        is Piece.Prose -> Markdown(
            piece.text,
            query,
            Modifier.fillMaxWidth(),
            breaks = if (hand) Breaks.Hard else Breaks.Soft,
        )
        is Piece.Attach -> AttachmentCard(piece.att, attachments, Modifier.fillMaxWidth())
        is Piece.Picture -> InlinePicture(piece.att, attachments, Modifier.fillMaxWidth())
        is Piece.Sound -> FileAffordance(piece.path, attachments, Modifier.fillMaxWidth())
        is Piece.Fence -> CodeCard(piece.lang, piece.text, query)
        is Piece.Patch -> DiffCard(piece.path, piece.text, query, attachments = attachments)
        is Piece.Launch -> Surface(
            Modifier.fitContent(),
            background = Kampr.tokens.color.raise,
            radius = Kampr.tokens.radii.md,
        ) { SubCard(piece.sub) }
        is Piece.Call -> {
            val key = "$turnId#$index"
            ToolCard(
                tool = piece.tool,
                detail = piece.detail,
                query = query,
                expanded = key in expanded,
                onToggle = { onToggle(key) },
                sub = piece.sub,
                attachments = attachments,
                output = piece.output,
            )
        }
    }
}
