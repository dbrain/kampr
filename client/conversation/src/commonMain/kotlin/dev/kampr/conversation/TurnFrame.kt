package dev.kampr.conversation

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.wire.Turn

// The third one is nobody: a compaction summary is filed under a user record and neither spoken
// nor typed, and calling it either of the other two is the lie this exists to stop. The fourth is
// a person again — whoever typed it — before the harness has started on it.
enum class Speaker { You, Queued, Agent, Summary }

fun speakerOf(turn: Turn): Speaker = when {
    isSummary(turn) -> Speaker.Summary
    isQueued(turn) -> Speaker.Queued
    turn.role == "user" -> Speaker.You
    else -> Speaker.Agent
}

// What tells the two apart, and it has to survive four themes. Not a pair of status colours:
// `accent` and `working` are the *same colour* in Phosphor and in Warm, and `done` is the same as
// `text` in Brutalist — a scheme built on two hues reads as one in half the themes shipped. So the
// speaker the reader is carries the accent and the other carries the quiet ink, which is a
// distinction every palette here keeps, and Brutalist — where `bg`, `surface` and `raise` are all
// one colour — still reads because the rail and the border carry it rather than the ground.
@Immutable
class SpeakerSkin(val rail: Color, val ground: Color, val label: String)

fun speakerSkin(tokens: KamprTokens, speaker: Speaker, agent: String?): SpeakerSkin {
    val color = tokens.color
    return when (speaker) {
        Speaker.You -> SpeakerSkin(
            rail = color.accent,
            ground = color.accent.copy(alpha = 0.07f).compositeOver(color.raise),
            label = "you",
        )
        // A person's message the harness has not reached. The rail keeps the accent because
        // somebody typed it; the ground drops to the quiet one and the label says outright that it
        // is still waiting, because a card that reads exactly like a turn the agent has taken up
        // moves the disagreement between the two surfaces rather than ending it. Not "you": the
        // queue is the pane's, and a prompt in it may have been sent from the desk.
        Speaker.Queued -> SpeakerSkin(color.accent, color.raise, "queued")
        Speaker.Agent -> SpeakerSkin(color.dim, color.surface, agent ?: "agent")
        // Quieter than either speaker in every theme, because it is the one row on the screen that
        // nobody said — and `raise` separates it from a reply's ground without a hue that would
        // read as a status somewhere.
        Speaker.Summary -> SpeakerSkin(color.mute, color.raise, "compacted")
    }
}

@Composable
fun speakerSkin(speaker: Speaker, agent: String?): SpeakerSkin = speakerSkin(Kampr.tokens, speaker, agent)

// Where a piece sits in the box its block is drawn as. A block is one box however many pieces it is
// made of, and the pieces are separate list items on purpose: a reply of a hundred steps composed
// whole is a hundred tool cards laid out to show four of them.
enum class BlockEdge { Head, Middle, Foot, Only }

fun blockEdge(before: String?, block: String, after: String?): BlockEdge = when {
    before != block && after != block -> BlockEdge.Only
    before != block -> BlockEdge.Head
    after != block -> BlockEdge.Foot
    else -> BlockEdge.Middle
}

private val RAIL = 3.dp
val BLOCK_GAP = 14.dp

// One box, drawn a piece at a time. Each piece paints the *whole* block's rounded rectangle with
// its top pushed above the piece and its bottom below it, and clips to its own bounds — so a head
// keeps the top corners, a foot keeps the bottom ones, and the pieces between draw two straight
// sides that meet with nothing between them. The alternative is per-edge line drawing, which has
// to reinvent the corner arcs and gets them subtly wrong at every radius the four themes use.
//
// The gap that separates one block from the next is paid by the foot, outside the paint, because
// a gap inside the box is a gap in the box.
@Composable
fun BlockFrame(
    skin: SpeakerSkin,
    edge: BlockEdge,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    val tokens = Kampr.tokens
    val border = tokens.card
    val head = edge == BlockEdge.Head || edge == BlockEdge.Only
    val foot = edge == BlockEdge.Foot || edge == BlockEdge.Only
    Box(
        modifier
            .fillMaxWidth()
            .padding(bottom = if (foot) BLOCK_GAP else 0.dp)
            .clipToBounds()
            .drawBehind {
                val radius = tokens.radii.md.toPx()
                val stroke = border.width.toPx()
                val top = if (head) 0f else -radius * 2f
                val bottom = if (foot) size.height else size.height + radius * 2f
                val path = Path().apply {
                    addRoundRect(
                        RoundRect(
                            left = 0f,
                            top = top,
                            right = size.width,
                            bottom = bottom,
                            cornerRadius = CornerRadius(radius),
                        ),
                    )
                }
                drawPath(path, skin.ground)
                clipPath(path) { drawRect(skin.rail, size = Size(RAIL.toPx(), size.height)) }
                if (border.visible) {
                    drawPath(
                        Path().apply {
                            addRoundRect(
                                RoundRect(
                                    left = stroke / 2f,
                                    top = top + stroke / 2f,
                                    right = size.width - stroke / 2f,
                                    bottom = bottom - stroke / 2f,
                                    cornerRadius = CornerRadius(radius),
                                ),
                            )
                        },
                        border.color,
                        style = Stroke(stroke),
                    )
                }
            },
    ) {
        Column(
            Modifier
                .fillMaxWidth()
                .padding(
                    start = RAIL + 11.dp,
                    end = 13.dp,
                    top = if (head) 8.dp else 0.dp,
                    bottom = if (foot) 10.dp else 10.dp,
                ),
            verticalArrangement = Arrangement.spacedBy(8.dp),
            content = content,
        )
    }
}

