package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.PaneInfo

object FallbackSurfaces : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Unit

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) {
        val tokens = Kampr.tokens
        Column(
            modifier
                .background(tokens.color.bg)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(13.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                LabelText("transcript", tokens.type.metaSmall, tokens.color.mute)
                Divider(Modifier.weight(1f).height(1.dp))
                KText("${pane.turns.size} turns", tokens.type.meta, tokens.color.mute)
            }
            for (turn in pane.turns) {
                // A compaction summary is written as a `user` record by the harness and is not the
                // operator speaking. This surface has no fold to put it behind, so the least it can
                // do is not put it in their bubble.
                if (turn.role == "user" && turn.kind != "compact") {
                    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
                        Surface(
                            Modifier.widthIn(max = 460.dp),
                            background = tokens.color.raise,
                            radius = tokens.radii.md,
                        ) {
                            Column(Modifier.padding(horizontal = 13.dp, vertical = 9.dp)) {
                                for (block in turn.blocks) BlockView(block)
                            }
                        }
                    }
                } else {
                    Column(verticalArrangement = Arrangement.spacedBy(11.dp)) {
                        for (block in turn.blocks) BlockView(block)
                    }
                }
            }
        }
    }

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Unit
}

@Composable
private fun BlockView(block: Block) {
    val tokens = Kampr.tokens
    when (block) {
        is Block.Md -> KText(block.text, tokens.type.body, tokens.color.text, maxLines = 40)
        is Block.Code -> Surface(
            Modifier.fillMaxWidth(),
            background = tokens.color.surface2,
            radius = tokens.radii.md,
        ) {
            Column {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 11.dp, vertical = 6.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    KText(block.lang ?: "text", tokens.type.meta, tokens.color.mute)
                    KText("Copy", tokens.type.micro, tokens.color.dim)
                }
                Divider(Modifier.fillMaxWidth().height(1.dp))
                KText(
                    block.text,
                    tokens.type.meta.copy(fontFamily = tokens.fonts.mono),
                    tokens.color.text,
                    Modifier.padding(horizontal = 11.dp, vertical = 9.dp),
                    maxLines = 24,
                )
            }
        }
        is Block.Tool -> Surface(
            Modifier.fillMaxWidth(),
            radius = tokens.radii.md,
        ) {
            Row(
                Modifier.padding(horizontal = 12.dp, vertical = 9.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(9.dp),
            ) {
                IconGlyph(KamprIcons.tool, 14.dp, tokens.color.dim)
                KText(
                    listOfNotNull(block.name, block.summary).joinToString(" · "),
                    tokens.type.meta,
                    tokens.color.dim,
                    Modifier.weight(1f),
                )
                block.lines?.let { KText("$it lines", tokens.type.micro, tokens.color.mute) }
                IconGlyph(KamprIcons.chevronRight, 12.dp, tokens.color.mute)
            }
        }
        is Block.Diff -> Surface(
            Modifier.fillMaxWidth(),
            background = tokens.color.surface2,
            radius = tokens.radii.md,
        ) {
            KText(
                block.text,
                tokens.type.meta.copy(fontFamily = tokens.fonts.mono),
                tokens.color.done,
                Modifier.padding(11.dp),
                maxLines = 24,
            )
        }
        // The fallback surface has no way to open one, and the tool card above it already says
        // an agent was launched.
        is Block.Sub -> Unit
        is Block.Unknown -> Unit
    }
}

