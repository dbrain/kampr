package dev.kampr.conversation

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.util.clockFace
import dev.kampr.shared.util.clockTime
import dev.kampr.shared.util.localDay
import dev.kampr.shared.util.isoIsZoned
import dev.kampr.shared.util.localOffsetMillis
import dev.kampr.shared.util.parseIsoMillis
import dev.kampr.shared.util.relativeTime
import dev.kampr.shared.wire.Turn

// Folded by key, in the same set the tool runs and the tool cards toggle through, so what the
// reader put away stays away across every tick of the transcript under it.
fun foldKey(turn: Turn): String? = if (foldable(turn)) "fold:${turn.id}" else null

// A header is a row of chrome carrying a 36 dp target, so it has to buy more than it costs, which
// is the same test the tool runs get. A turn of nothing but calls is the run's business and wears
// a chevron already; a preview is being rewritten under the reader and folding what is changing is
// the one thing a fold must not do. What is left earns a chevron once it runs past a couple of
// lines or holds more than one thing.
//
// A reply used to be excluded outright, on the grounds that it was short by construction and said
// who wrote it by sitting in its own gutter on the right. Neither holds now that a turn is a
// full-width card whoever wrote it, and a pasted stack trace is a reply — so the size test decides
// for both speakers.
fun foldable(turn: Turn): Boolean {
    if (turn.id == LIVE_TURN_ID) return false
    val pieces = groupBlocks(turn.blocks)
    if (pieces.isEmpty() || pieces.all { it is Piece.Call }) return false
    return pieces.size > 1 || turnText(turn).count { it == '\n' } >= 2
}

// A time of day, in the reader's own zone, because every harness Kampr reads writes UTC and says
// so with a `Z` (#285) — which makes `at` an absolute instant rather than the floating local time
// this used to assume. An age was the honest reading while the offset was unknown and it is the
// weaker one now: it goes stale where it is painted, "now" beside an hour-old message is what a
// stopped ticker says, and it cannot be lined up against anything the operator saw elsewhere.
//
// A stamp that names no zone still gets an age. No adapter in tree writes one, and the day it does
// is not the day to start guessing an offset.
fun turnStamp(at: String?, nowMillis: Double): String? {
    val millis = parseIsoMillis(at) ?: return null
    if (!isoIsZoned(at)) return relativeTime(at, nowMillis)
    return clockFace(millis, nowMillis, localOffsetMillis(millis))
}

// When a reply started and when it stopped. The second half is a bare face — the day is already
// in front of the first one, and "16 Aug 13:30 → 16 Aug 14:02" says the date twice for one
// afternoon. A reply of a single record, or one whose records all landed inside a minute, has
// nothing to put on the right of an arrow and does not draw one.
fun replySpan(from: String?, to: String?, nowMillis: Double): String? {
    val opened = turnStamp(from, nowMillis) ?: return null
    val began = parseIsoMillis(from) ?: return opened
    val ended = parseIsoMillis(to) ?: return opened
    if (!isoIsZoned(to)) return opened
    val offset = localOffsetMillis(ended)
    val closed = clockTime(ended, offset)
    if (closed == clockTime(began, localOffsetMillis(began))) return opened
    return when (localDay(began, localOffsetMillis(began))) {
        localDay(ended, offset) -> "$opened → $closed"
        else -> "$opened → ${turnStamp(to, nowMillis)}"
    }
}

private val DECORATION = Regex("^(#{1,6}|>+|[-*+]|\\d+\\.)\\s+")

// One line of prose for a folded turn to show of itself. Markdown's own punctuation is stripped
// rather than rendered: a row reading "## Corrections and event" is the syntax, not the message.
fun turnGist(turn: Turn): String =
    turnText(turn)
        .lineSequence()
        .map { it.trim() }
        .firstOrNull { it.isNotEmpty() }
        ?.replace(DECORATION, "")
        ?.replace("**", "")
        ?.replace("`", "")
        ?.trim()
        .orEmpty()

fun headerLabel(skin: SpeakerSkin, stamp: String?, gist: String, parts: Int): String = listOfNotNull(
    skin.label,
    stamp,
    gist.takeIf { it.isNotEmpty() },
    if (parts > 1) "$parts parts" else null,
).joinToString(", ")

// Who spoke, when, and — only when there is something to put away — the chevron that does it. A
// turn that cannot be folded still gets the first two: the reader asked for the time on every
// message, and a line of text is not the 36 dp of touch target a control would cost.
@Composable
fun TurnHeader(
    skin: SpeakerSkin,
    stamp: String?,
    gist: String,
    parts: Int,
    folded: Boolean,
    onToggle: (() -> Unit)?,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val held = headerLabel(skin, stamp, gist, parts)
    DisableSelection {
        Row(
            modifier
                .fillMaxWidth()
                .then(
                    if (onToggle == null) Modifier
                    else Modifier
                        .touchable(LANDSCAPE_TOUCH)
                        .action(
                            if (folded) "Show the message of $held" else "Hide the message of $held",
                            onToggle,
                            selected = !folded,
                        ),
                ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            LabelText(skin.label, tokens.type.metaSmall, skin.rail)
            if (stamp != null) KText(stamp, tokens.type.micro, tokens.color.mute)
            if (folded) KText(gist, tokens.type.meta, tokens.color.dim, Modifier.weight(1f))
            else Box(Modifier.weight(1f))
            if (onToggle != null) {
                IconGlyph(
                    if (folded) ConversationIcons.chevronDown else ConversationIcons.chevronUp,
                    12.dp,
                    tokens.color.mute,
                )
            }
        }
    }
}
