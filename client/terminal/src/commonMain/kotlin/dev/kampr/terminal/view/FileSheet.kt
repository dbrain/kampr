package dev.kampr.terminal.view

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.absolutePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.CloseAction
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edgeBottom
import dev.kampr.shared.ui.modal
import dev.kampr.shared.ui.touchable
import dev.kampr.terminal.file.Peeked
import dev.kampr.terminal.file.fileName

// How much of a file is laid out at once. The route will hand back 8 MiB, and this surface shapes
// it into one text node on a phone that is already holding a scrollback and a run-layout cache.
private const val LINES_SHOWN = 2_000

// A path on the grid is a file the operator can already `cat` on their own machine, and this is
// where the bytes the node hands back are read. Deliberately not the grid: nothing here writes a
// cell, asks for a size or touches the PTY — it covers the pane and goes away again.
//
// `chromeTop` and `chromeBottom` are the pane's own furniture, and they are the whole reason this
// sheet takes them: the pane header is painted *over* this surface, not above it, so a header row
// laid out at y=0 is a header row nobody can see or press. Escape closed the sheet and nothing
// else did.
@Composable
fun FileSheet(
    path: String,
    state: Peeked,
    onClose: () -> Unit,
    onCopy: (String) -> Unit,
    chromeTop: Dp,
    chromeBottom: Dp,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    val words = (state as? Peeked.Words)?.text
    Column(
        modifier
            .fillMaxSize()
            .background(tokens.color.bg)
            .modal(onClose),
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .background(tokens.color.bar)
                .edgeBottom()
                .absolutePadding(
                    left = 12.dp + safe.left,
                    right = 4.dp + safe.right,
                    top = 8.dp + chromeTop,
                    bottom = 8.dp,
                ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Column(Modifier.weight(1f)) {
                KText(fileName(path), tokens.type.meta, tokens.color.text)
                KText(path, tokens.type.micro, tokens.color.mute)
            }
            if (words != null) CopyAll(path, words, onCopy)
            CloseAction("Close $path", onClose)
        }

        Box(Modifier.weight(1f).fillMaxWidth()) {
            when (state) {
                Peeked.Fetching -> Note("asking the node for $path", tokens.color.working)
                is Peeked.Failed -> Note(state.reason, tokens.color.blocked)
                is Peeked.Saved -> Note("saved to ${state.where}", tokens.color.done)
                is Peeked.Picture -> Image(
                    bitmap = state.image,
                    contentDescription = fileName(path),
                    modifier = Modifier.fillMaxSize().padding(12.dp),
                    contentScale = ContentScale.Fit,
                )
                is Peeked.Words -> Words(state.text, chromeBottom)
            }
        }
    }
}

// What the button takes is the whole file, including the lines the layout cap is not drawing — so
// the label says how many, every time. A copy that quietly stops at the two-thousandth line of a
// file the operator has scrolled to the end of is the worst of the three possible behaviours.
@Composable
private fun CopyAll(path: String, text: String, onCopy: (String) -> Unit) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    val lines = remember(text) { text.count { it == '\n' } + 1 }
    Box(
        Modifier
            .touchable()
            .background(tokens.color.surface, shape)
            .action(copyLabel(path, lines), { onCopy(text) }, shape)
            .padding(horizontal = 14.dp, vertical = 9.dp),
        contentAlignment = Alignment.Center,
    ) {
        KText("Copy", tokens.type.buttonSmall, tokens.color.text)
    }
}

private fun copyLabel(path: String, lines: Int): String =
    if (lines > LINES_SHOWN) "Copy all $lines lines of $path, including the ones not shown"
    else "Copy $path"

@Composable
private fun Note(words: String, tint: Color) {
    KText(
        words,
        Kampr.tokens.type.caption,
        tint,
        Modifier.fillMaxWidth().padding(16.dp).announce(words),
        maxLines = 4,
    )
}

// Monospace and unwrapped, because a file whose lines are code reads as code only where the
// columns line up. The horizontal scroll is the price, and it is the same one the grid pays.
//
// In a `SelectionContainer` because a viewer nothing can be taken out of is a dead end: `KText`
// draws a `BasicText`, which is inert until something above it says otherwise. The button beside
// the title is for the whole file; this is for the three lines the operator actually wanted.
@Composable
private fun Words(text: String, chromeBottom: Dp) {
    val tokens = Kampr.tokens
    val lines = remember(text) { text.count { it == '\n' } + 1 }
    val shown = remember(text) {
        if (lines <= LINES_SHOWN) text else text.lineSequence().take(LINES_SHOWN).joinToString("\n")
    }
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .horizontalScroll(rememberScrollState())
            .padding(start = 12.dp, end = 12.dp, top = 12.dp, bottom = 12.dp + chromeBottom),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        SelectionContainer {
            KText(shown, tokens.type.meta, tokens.color.text, maxLines = LINES_SHOWN)
        }
        if (lines > LINES_SHOWN) {
            KText(
                "showing the first $LINES_SHOWN of $lines lines — Copy takes all $lines",
                tokens.type.micro,
                tokens.color.mute,
                Modifier.announce("Showing the first $LINES_SHOWN of $lines lines. Copy takes all $lines."),
            )
        }
    }
}
