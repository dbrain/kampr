package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr

private val SHEET_MAX_WIDTH = 420.dp
private const val SCRIM_ALPHA = 0.72f

// The scrim is the theme's own ground at partial opacity rather than a colour of its own, so it
// stays right on a black brutalist ground and on a light one.
@Composable
fun Scrim(onDismiss: () -> Unit, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val interaction = remember { MutableInteractionSource() }
    Box(
        modifier
            .fillMaxSize()
            .background(tokens.color.bg.copy(alpha = SCRIM_ALPHA))
            .clickable(interaction, indication = null, onClick = onDismiss)
    )
}

@Composable
fun selectedEdge(): BorderSpec =
    BorderSpec(if (Kampr.tokens.card.visible) Kampr.tokens.card.width else Kampr.tokens.chrome.width, Kampr.tokens.color.accent)

@Composable
fun BottomSheet(
    breakpoint: Breakpoint,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    val tokens = Kampr.tokens
    Box(modifier.fillMaxSize()) {
        Scrim(onDismiss)
        BoxWithConstraints(
            Modifier.align(if (breakpoint == Breakpoint.Portrait) Alignment.BottomCenter else Alignment.Center)
        ) {
            val width = if (maxWidth < SHEET_MAX_WIDTH) maxWidth else SHEET_MAX_WIDTH
            val tall = maxHeight * (if (breakpoint == Breakpoint.Portrait) 0.92f else 0.94f)
            val shape = RoundedCornerShape(tokens.radii.lg)
            Column(
                Modifier
                    .width(width)
                    .heightIn(max = tall)
                    .background(tokens.color.bar, shape)
                    .edge(tokens.chrome, shape),
                content = content,
            )
        }
    }
}

@Composable
fun SheetHeader(
    title: String,
    subtitle: String?,
    onBack: (() -> Unit)?,
    onClose: () -> Unit,
) {
    val tokens = Kampr.tokens
    Column {
        Row(
            Modifier.fillMaxWidth().padding(start = 20.dp, top = 18.dp, end = 20.dp, bottom = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (onBack != null) {
                IconGlyph(KamprIcons.chevronLeft, 16.dp, tokens.color.dim, Modifier.clickable(onClick = onBack))
            }
            KText(title, tokens.type.screenTitle, tokens.color.text, Modifier.weight(1f))
            IconGlyph(KamprIcons.cross, 18.dp, tokens.color.dim, Modifier.clickable(onClick = onClose))
        }
        if (subtitle != null) {
            Box(Modifier.padding(start = 20.dp, end = 20.dp, bottom = 9.dp)) {
                LabelText(subtitle, tokens.type.micro, tokens.color.mute)
            }
        }
    }
}

@Composable
fun SheetSection(label: String) {
    val tokens = Kampr.tokens
    Box(Modifier.padding(start = 20.dp, top = 20.dp, end = 20.dp, bottom = 9.dp)) {
        LabelText(label, tokens.type.micro, tokens.color.mute)
    }
}

@Composable
fun SheetCard(
    icon: Icon?,
    iconTint: androidx.compose.ui.graphics.Color?,
    title: String,
    subtitle: String,
    subtitleMono: Boolean = false,
    selected: Boolean = false,
    onClick: (() -> Unit)? = null,
    trailing: @Composable (() -> Unit)? = null,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.lg)
    Row(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.surface, shape)
            .edge(if (selected) selectedEdge() else tokens.card, shape)
            .let { if (onClick != null) it.clickable(onClick = onClick) else it }
            .padding(horizontal = 15.dp, vertical = 13.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (icon != null) {
            Badge(36.dp, 17.dp, icon, iconTint ?: tokens.color.accent)
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            KText(title, tokens.type.bodyStrong, tokens.color.text)
            KText(
                subtitle,
                if (subtitleMono) tokens.type.meta else tokens.type.captionSmall,
                tokens.color.mute,
                maxLines = 2,
            )
        }
        if (trailing != null) trailing() else if (onClick != null) {
            IconGlyph(KamprIcons.chevronRight, 13.dp, tokens.color.mute)
        }
    }
}

@Composable
fun Chip(text: String, selected: Boolean, onClick: () -> Unit, quiet: Boolean = false) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Box(
        Modifier
            .background(if (selected) tokens.color.accent else tokens.color.raise, shape)
            .edge(if (selected) tokens.card else tokens.card, shape)
            .clickable(onClick = onClick)
            .padding(horizontal = 13.dp, vertical = 7.dp),
    ) {
        KText(
            text,
            tokens.type.pill,
            when {
                selected -> tokens.color.onAccent
                quiet -> tokens.color.dim
                else -> tokens.color.text
            },
        )
    }
}
