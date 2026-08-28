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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.CloseAction
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edgeBottom
import dev.kampr.shared.ui.modal
import dev.kampr.terminal.file.Peeked
import dev.kampr.terminal.file.fileName

// How much of a file is laid out at once. The route will hand back 8 MiB, and this surface shapes
// it into one text node on a phone that is already holding a scrollback and a run-layout cache.
private const val LINES_SHOWN = 2_000

// A path on the grid is a file the operator can already `cat` on their own machine, and this is
// where the bytes the node hands back are read. Deliberately not the grid: nothing here writes a
// cell, asks for a size or touches the PTY — it covers the pane and goes away again.
@Composable
fun FileSheet(path: String, state: Peeked, onClose: () -> Unit, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
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
                .padding(
                    start = 12.dp + safe.left,
                    end = 4.dp + safe.right,
                    top = 8.dp + safe.top,
                    bottom = 8.dp,
                ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Column(Modifier.weight(1f)) {
                KText(fileName(path), tokens.type.meta, tokens.color.text)
                KText(path, tokens.type.micro, tokens.color.mute)
            }
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
                is Peeked.Words -> Words(state.text)
            }
        }
    }
}

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
@Composable
private fun Words(text: String) {
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
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        KText(shown, tokens.type.meta, tokens.color.text, maxLines = LINES_SHOWN)
        if (lines > LINES_SHOWN) {
            KText(
                "showing the first $LINES_SHOWN of $lines lines",
                tokens.type.micro,
                tokens.color.mute,
                Modifier.announce("Showing the first $LINES_SHOWN of $lines lines"),
            )
        }
    }
}
