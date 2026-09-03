package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.touchable
import dev.kampr.terminal.render.Target
import dev.kampr.terminal.render.TargetKind

// A detected URL is not a declared one. Pane output is attacker-influenceable, so the target is
// shown and acted on deliberately rather than navigated to on touch (probes #36/#37).
//
// Which is also why the words are never ellipsised. This is the one place an operator reads what
// they are about to open, and `https://herdr.dev.evil.example/…` truncated to `https://herdr.dev…`
// is not a shorter label, it is a different address. It wraps and, past the height a card may
// take, it scrolls; nothing about it is allowed to end in an ellipsis.
private const val WRAPPED_LINES = 40

private val CARD_MAX_WIDTH = 460.dp
private val WORDS_MAX_HEIGHT = 168.dp

// The bottom strip, which is what a screen too short to put a card beside the tap still wants: on
// a phone an affordance anchored under the thumb that raised it is an affordance nobody can read.
@Composable
fun TargetStrip(
    target: Target,
    onAct: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier,
) {
    Row(
        modifier
            .fillMaxWidth()
            .announce(spoken(target))
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Words(target, onDismiss, Modifier.weight(1f))
        ActButton(target, onAct)
    }
}

// The same offer, put where the operator was looking. A desk clicks a path on the first line of a
// tall pane and reads the answer eight hundred pixels away with nothing saying which of the paths
// on screen it belongs to; `atPixels` is the placement `GridMenu` already uses, and it holds the
// card inside the surface when the click lands near an edge.
@Composable
internal fun TargetCard(
    target: Target,
    at: Offset,
    onAct: () -> Unit,
    onDismiss: () -> Unit,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Column(
        Modifier
            .atPixels(at.x, at.y)
            .widthIn(max = CARD_MAX_WIDTH)
            .background(tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .announce(spoken(target))
            .padding(10.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Words(target, null, Modifier)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            ActButton(target, onAct)
            Box(
                Modifier
                    .touchable()
                    .background(tokens.color.surface, shape)
                    .edge(tokens.card, shape)
                    .action("Dismiss ${kind(target)} ${target.text}", onDismiss, shape)
                    .padding(horizontal = 14.dp, vertical = 11.dp),
                contentAlignment = Alignment.Center,
            ) {
                KText("Dismiss", tokens.type.buttonSmall, tokens.color.dim)
            }
        }
    }
}

// The words are their own dismiss target on the strip, where the only other thing to press is
// Open and a mis-tap on it opens something. The card carries a Dismiss of its own, so there the
// words are just words — two controls with the same label is one the operator cannot name.
@Composable
private fun Words(target: Target, onDismiss: (() -> Unit)?, modifier: Modifier) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Row(
        modifier
            .background(tokens.color.surface, shape)
            .edge(tokens.card, shape)
            .then(
                if (onDismiss == null) {
                    Modifier
                } else {
                    Modifier.touchable().action("Dismiss ${kind(target)} ${target.text}", onDismiss, shape)
                },
            )
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        KText(word(target.kind), tokens.type.metaSmall, tokens.color.mute)
        Box(Modifier.weight(1f, fill = false).heightIn(max = WORDS_MAX_HEIGHT).verticalScroll(rememberScrollState())) {
            KText(target.text, tokens.type.caption, tokens.color.text, maxLines = WRAPPED_LINES)
        }
    }
}

@Composable
private fun ActButton(target: Target, onAct: () -> Unit) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Box(
        Modifier
            .touchable()
            .background(tokens.color.accent, shape)
            .action(actLabel(target), onAct, shape)
            .padding(horizontal = 16.dp, vertical = 11.dp),
        contentAlignment = Alignment.Center,
    ) {
        KText(actWord(target), tokens.type.buttonSmall, tokens.color.onAccent)
    }
}

private fun kind(target: Target): String = when (target.kind) {
    TargetKind.Link -> "Declared link"
    TargetKind.Url -> "Detected address"
    TargetKind.Path -> "File path"
    TargetKind.File -> "File on this machine"
}

private fun word(kind: TargetKind): String = when (kind) {
    TargetKind.Link -> "link"
    TargetKind.Url -> "detected"
    TargetKind.Path -> "path"
    TargetKind.File -> "file"
}

private fun spoken(target: Target): String = "${kind(target)}: ${target.text}"

// A reference nothing can resolve is copied; everything else is opened, and a file is opened by
// the node handing back the bytes rather than by this device going and looking for them.
private fun actWord(target: Target): String = if (target.kind == TargetKind.Path) "Copy" else "Open"

private fun actLabel(target: Target): String = "${actWord(target)} ${target.text}"
