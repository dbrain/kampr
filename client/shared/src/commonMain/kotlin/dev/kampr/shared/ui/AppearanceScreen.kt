package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.AllThemes
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.KamprTheme
import dev.kampr.shared.theme.ThemeId
import dev.kampr.shared.theme.ThemeMode
import dev.kampr.shared.theme.ThemeSpec
import dev.kampr.shared.theme.TypeScale

private val GRID_GAP = 14.dp

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun AppearanceScreen(
    selected: ThemeId,
    mode: ThemeMode,
    onSelect: (ThemeId) -> Unit,
    onMode: (ThemeMode) -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    BoxWithConstraints(modifier.fillMaxSize().background(tokens.color.bg)) {
        // A theme card is a card, not a window: past its measure the swatches, the key caps and
        // the specimen row are all stretch, and under it the credit line wraps. The grid keeps the
        // width it needs and takes the middle of whatever is left.
        //
        // The count comes off the window rather than off the breakpoint, because the breakpoint is
        // not the width this screen gets: a 900 dp desktop is about 600 dp here once the sidebar
        // has had its share, and four columns of it is 129 dp a card.
        val available = maxWidth - 40.dp
        val fitted = columnPlan(available, GRID_GAP, AllThemes.size, min = THEME_COLUMN_MIN).count
        // Rendered before it was believed: three columns of four themes is a lone card with a hole
        // beside it, which reads as broken next to the same four in a row or in a square. So the
        // count steps down off a last row of one — not down to an exact divisor, which would put a
        // fifth theme in a single column for ever. `count = 1` always satisfies it, so this ends.
        val columns = generateSequence(fitted) { it - 1 }.first { AllThemes.size % it != 1 }
        val card = columnWidth(available, GRID_GAP, columns)
        val grid = card * columns + GRID_GAP * (columns - 1)
        Column(Modifier.fillMaxSize(), horizontalAlignment = Alignment.CenterHorizontally) {
            // The header is given the grid's width rather than the window's, so that on a wide
            // monitor it arrives with the thing it heads instead of staying at an edge 660 dp
            // away — the same rule the setup screen's greeting was already given.
            //
            // Flowing rather than a Row: the caption is a whole sentence sharing a line with a
            // back arrow and a screen title, and with a weight it was handed 87 dp on a 600 dp
            // body and ellipsised mid-word. A sentence squeezed to nothing is not a sentence.
            FlowRow(
                Modifier.width(grid).padding(top = 18.dp, bottom = 14.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                itemVerticalAlignment = Alignment.CenterVertically,
            ) {
                BackAction("Back", onBack)
                KText("One token layer, four skins", tokens.type.screenTitle, tokens.color.text, Modifier.asHeading())
                KText(
                    "Soft native ships. The rest stay one attribute away.",
                    tokens.type.caption,
                    tokens.color.dim,
                )
            }
            Column(
                Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(14.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                GroundPicker(mode, onMode, Modifier.width(grid))
                AllThemes.chunked(columns).forEach { row ->
                    Row(Modifier.width(grid), horizontalArrangement = Arrangement.spacedBy(GRID_GAP)) {
                        for (spec in row) {
                            Box(Modifier.width(card)) {
                                ThemeCard(spec, spec.id == selected) { onSelect(spec.id) }
                            }
                        }
                        repeat(columns - row.size) { Box(Modifier.width(card)) }
                    }
                }
                Box(Modifier.height(20.dp))
            }
        }
    }
}

@Composable
private fun GroundPicker(mode: ThemeMode, onMode: (ThemeMode) -> Unit, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        LabelText("Ground", tokens.type.sectionLabel, tokens.color.mute)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            for (option in ThemeMode.entries) {
                val active = option == mode
                Box(
                    Modifier
                        .weight(1f)
                        .background(if (active) tokens.color.accent else tokens.color.raise, shape)
                        .edge(tokens.card, shape)
                        .touchable()
                        .action(
                            "${option.title} ground",
                            { onMode(option) },
                            shape,
                            role = Role.RadioButton,
                            selected = active,
                        )
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
                .action(
                    "${spec.id.title} theme — ${spec.id.credit}",
                    onSelect,
                    shape,
                    role = Role.RadioButton,
                    selected = active,
                )
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
