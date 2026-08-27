package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.ui.edge
import dev.kampr.shared.wire.Turn

enum class Speaker { You, Agent }

fun speakerOf(turn: Turn): Speaker = if (turn.role == "user") Speaker.You else Speaker.Agent

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
        Speaker.Agent -> SpeakerSkin(color.dim, color.surface, agent ?: "agent")
    }
}

@Composable
fun speakerSkin(speaker: Speaker, agent: String?): SpeakerSkin = speakerSkin(Kampr.tokens, speaker, agent)

private val RAIL = 3.dp

// A turn is a card with its speaker's colour down the leading edge. The rail is painted rather
// than laid out: a `fillMaxHeight` sibling inside a lazy list is measured against an unbounded
// height, and `IntrinsicSize.Min` would force an intrinsic pass over every line of markdown in
// the turn to place three device pixels.
@Composable
fun TurnFrame(
    skin: SpeakerSkin,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Box(
        modifier
            .fillMaxWidth()
            .background(skin.ground, shape)
            .edge(tokens.card, shape)
            .clip(shape)
            .drawBehind { drawRect(skin.rail, size = Size(RAIL.toPx(), size.height)) },
    ) {
        Column(
            Modifier.fillMaxWidth().padding(start = RAIL + 11.dp, end = 13.dp, top = 7.dp, bottom = 9.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
            content = content,
        )
    }
}
