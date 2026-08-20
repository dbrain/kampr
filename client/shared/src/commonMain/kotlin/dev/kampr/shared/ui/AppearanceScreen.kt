package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.AllThemes
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.KamprTheme
import dev.kampr.shared.theme.ThemeId
import dev.kampr.shared.theme.ThemeMode
import dev.kampr.shared.theme.ThemeSpec
import dev.kampr.shared.theme.TypeScale

@Composable
fun AppearanceScreen(
    selected: ThemeId,
    mode: ThemeMode,
    columns: Int,
    onSelect: (ThemeId) -> Unit,
    onMode: (ThemeMode) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        Row(
            Modifier.fillMaxWidth().padding(start = 16.dp, top = 18.dp, end = 24.dp, bottom = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            IconGlyph(KamprIcons.chevronLeft, 17.dp, tokens.color.dim, Modifier.clickable(onClick = onBack))
            KText("One token layer, four skins", tokens.type.screenTitle, tokens.color.text)
            KText(
                "Soft native ships. The rest stay one attribute away.",
                tokens.type.caption,
                tokens.color.dim,
                Modifier.weight(1f),
            )
        }
        Column(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 20.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            GroundPicker(mode, onMode)
            AllThemes.chunked(columns).forEach { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                    for (spec in row) {
                        Box(Modifier.weight(1f)) {
                            ThemeCard(spec, spec.id == selected) { onSelect(spec.id) }
                        }
                    }
                    repeat(columns - row.size) { Box(Modifier.weight(1f)) }
                }
            }
            Box(Modifier.height(20.dp))
        }
    }
}

@Composable
private fun GroundPicker(mode: ThemeMode, onMode: (ThemeMode) -> Unit) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        LabelText("Ground", tokens.type.sectionLabel, tokens.color.mute)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            for (option in ThemeMode.entries) {
                val active = option == mode
                Box(
                    Modifier
                        .weight(1f)
                        .background(if (active) tokens.color.accent else tokens.color.raise, shape)
                        .edge(tokens.card, shape)
                        .clickable { onMode(option) }
                        .padding(vertical = 10.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    LabelText(
                        option.title,
                        tokens.type.buttonSmall,
                        if (active) tokens.color.onAccent else tokens.color.text,
                    )
                }
            }
        }
    }
}

@Composable
private fun ThemeCard(spec: ThemeSpec, active: Boolean, onSelect: () -> Unit) {
    val outerAccent = Kampr.tokens.color.accent
    KamprTheme(spec, TypeScale.Desk) {
        val tokens = Kampr.tokens
        val shape = RoundedCornerShape(tokens.radii.lg)
        Column(
            Modifier
                .fillMaxWidth()
                .background(tokens.color.bg, shape)
                .border(if (active) 2.dp else 1.dp, if (active) outerAccent else tokens.color.line, shape)
                .clickable(onClick = onSelect)
                .padding(horizontal = 14.dp, vertical = 15.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                LabelText(spec.id.title, tokens.type.cardTitle.copy(fontSize = tokens.type.paneTitle.fontSize), tokens.color.text)
                KText("theme=\"${spec.id.key}\"", tokens.type.meta, tokens.color.mute)
            }
            Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                for (swatch in listOf(
                    tokens.color.surface,
                    tokens.color.dim,
                    tokens.color.accent,
                    tokens.color.blocked,
                    tokens.color.working,
                    tokens.color.done,
                )) {
                    Box(
                        Modifier
                            .weight(1f)
                            .height(22.dp)
                            .background(swatch, RoundedCornerShape(tokens.radii.sm))
                            .edge(tokens.card, RoundedCornerShape(tokens.radii.sm))
                    )
                }
            }
            Surface(Modifier.fillMaxWidth(), radius = tokens.radii.lg) {
                Row(
                    Modifier.padding(horizontal = 11.dp, vertical = 9.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(9.dp),
                ) {
                    Dot(tokens.color.blocked, 7.dp)
                    LabelText("kampr · claude", tokens.type.cardTitle, tokens.color.text, Modifier.weight(1f))
                    KText("now", tokens.type.micro, tokens.color.mute)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                KeyCap("esc", false, Modifier.weight(1f))
                KeyCap("ctrl", true, Modifier.weight(1f))
                KeyCap("alt", false, Modifier.weight(1f))
                KeyCap("tab", false, Modifier.weight(1f))
            }
            KText(spec.id.credit, tokens.type.micro, tokens.color.mute, maxLines = 2)
        }
    }
}

@Composable
private fun KeyCap(label: String, latched: Boolean, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Box(
        modifier
            .background(if (latched) tokens.color.accent else tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .padding(vertical = 9.dp),
        contentAlignment = Alignment.Center,
    ) {
        LabelText(label, tokens.type.key, if (latched) tokens.color.onAccent else tokens.color.text)
    }
}

