package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.Surface

enum class DiffKind { Added, Removed, Context, Hunk, Meta }

data class DiffLine(val kind: DiffKind, val text: String)

// Claude sends a unified diff rebuilt from `structuredPatch`; Codex sends `apply_patch` envelopes
// (`*** Begin Patch`). Both mark their lines with the same leading +/-/space, so one classifier
// covers them and the envelope lines become metadata rather than content.
fun parseDiff(text: String): List<DiffLine> = text.split('\n').map { line ->
    when {
        line.startsWith("***") || line.startsWith("+++") || line.startsWith("---") ->
            DiffLine(DiffKind.Meta, line)
        line.startsWith("@@") -> DiffLine(DiffKind.Hunk, line)
        line.startsWith("+") -> DiffLine(DiffKind.Added, line)
        line.startsWith("-") -> DiffLine(DiffKind.Removed, line)
        else -> DiffLine(DiffKind.Context, line)
    }
}

@Composable
fun DiffCard(path: String?, text: String, query: String, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val palette = rememberConversationPalette()
    val lines = remember(text) { parseDiff(text).dropLastWhile { it.text.isEmpty() } }
    val added = lines.count { it.kind == DiffKind.Added }
    val removed = lines.count { it.kind == DiffKind.Removed }
    val scroll = rememberScrollState()

    Surface(modifier.fillMaxWidth(), background = palette.codeGround, radius = tokens.radii.md) {
        Column {
            Row(
                Modifier.fillMaxWidth().background(palette.codeBar).padding(horizontal = 11.dp, vertical = 7.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                IconGlyph(ConversationIcons.diff, 12.dp, tokens.color.dim)
                KText(
                    path?.substringAfterLast('/') ?: "diff",
                    tokens.type.meta,
                    tokens.color.dim,
                    Modifier.weight(1f),
                )
                KText("+$added", tokens.type.micro, palette.added)
                KText("−$removed", tokens.type.micro, palette.removed)
            }
            Box(Modifier.fillMaxWidth().height(1.dp).background(palette.rule))
            Column(Modifier.fillMaxWidth().horizontalScroll(scroll).width(IntrinsicSize.Max)) {
                for (line in lines) DiffRow(line, query, palette)
            }
        }
    }
}

@Composable
private fun DiffRow(line: DiffLine, query: String, palette: ConversationPalette) {
    val tokens = Kampr.tokens
    val ground: Color? = when (line.kind) {
        DiffKind.Added -> palette.addedGround
        DiffKind.Removed -> palette.removedGround
        DiffKind.Hunk -> palette.hunkGround
        else -> null
    }
    val ink = when (line.kind) {
        DiffKind.Added -> palette.added
        DiffKind.Removed -> palette.removed
        DiffKind.Hunk -> palette.hunk
        DiffKind.Meta -> tokens.color.mute
        DiffKind.Context -> tokens.color.dim
    }
    val text = remember(line, query, palette) {
        androidx.compose.ui.text.AnnotatedString(line.text.ifEmpty { " " }).markMatches(query, palette.match)
    }
    BasicText(
        text = text,
        modifier = Modifier
            .fillMaxWidth()
            .let { if (ground != null) it.background(ground) else it }
            .padding(horizontal = 11.dp, vertical = 1.dp),
        style = tokens.type.caption.copy(fontFamily = tokens.fonts.mono, color = ink),
        softWrap = false,
    )
}
