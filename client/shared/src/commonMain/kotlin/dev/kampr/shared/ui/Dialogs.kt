package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.layout
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr

// True inside a sheet drawn as a centred dialog (landscape, desktop, browser) or a pointer's popover
// rather than a phone's bottom sheet, which is what decides how its buttons sit and how tall a row is.
val LocalSheetWide = compositionLocalOf { false }

private val WIDE_BUTTON = 112.dp

// The one way a sheet ends: dismiss first and the commit last, in the place the thumb or the
// pointer finds it every time. A phone splits the width between them; a dialog keeps them to the
// right at their own size. A commit that destroys something is red, never the accent.
@Composable
fun SheetActions(
    confirm: String,
    onConfirm: () -> Unit,
    modifier: Modifier = Modifier,
    onCancel: (() -> Unit)? = null,
    cancel: String = "Cancel",
    danger: Boolean = false,
    enabled: Boolean = true,
    confirmLabel: String? = null,
    extra: (@Composable RowScope.() -> Unit)? = null,
) {
    val wide = LocalSheetWide.current
    Row(
        modifier.fillMaxWidth().padding(start = 20.dp, top = 12.dp, end = 20.dp, bottom = 18.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp, if (wide) Alignment.End else Alignment.Start),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val sized = if (wide) Modifier.roomy() else Modifier.weight(1f)
        extra?.invoke(this)
        if (onCancel != null) QuietAction(cancel, onCancel, sized, style = Kampr.tokens.type.button, vertical = 13.dp)
        PrimaryAction(confirm, onConfirm, sized, vertical = 13.dp, enabled = enabled, label = confirmLabel, danger = danger)
    }
}

// A dialog's button is its caption's width plus a margin either side, never narrower than
// `WIDE_BUTTON`. The buttons pad only vertically, so the margin is added here as a wider measure.
private fun Modifier.roomy(): Modifier = layout { measurable, constraints ->
    val want = (measurable.maxIntrinsicWidth(constraints.maxHeight) + WIDE_PAD.roundToPx() * 2)
        .coerceAtLeast(WIDE_BUTTON.roundToPx())
        .coerceIn(constraints.minWidth, constraints.maxWidth)
    val placed = measurable.measure(constraints.copy(minWidth = want, maxWidth = want))
    layout(placed.width, placed.height) { placed.place(0, 0) }
}

private val WIDE_PAD = 20.dp

// A command in a menu or an actions sheet: what it does, with the glyph that says so at a glance.
// Destructive rows are red and belong last in their group, after a `MenuDivider`.
@Composable
fun MenuRow(
    text: String,
    onClick: () -> Unit,
    icon: Icon? = null,
    danger: Boolean = false,
    label: String = text,
    enabled: Boolean = true,
    detail: String? = null,
) {
    val tokens = Kampr.tokens
    val tone = when {
        !enabled -> tokens.color.mute
        danger -> tokens.color.blocked
        else -> tokens.color.text
    }
    Row(
        Modifier
            .fillMaxWidth()
            .touchable(if (LocalSheetWide.current) LANDSCAPE_TOUCH else TOUCH)
            .action(label, onClick, enabled = enabled)
            .padding(horizontal = 20.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Box(Modifier.defaultMinSize(minWidth = 18.dp), contentAlignment = Alignment.Center) {
            if (icon != null) IconGlyph(icon, 16.dp, if (danger && enabled) tokens.color.blocked else tokens.color.dim)
        }
        DisableSelection {
            KText(text, tokens.type.body, tone, Modifier.weight(1f))
            if (detail != null) KText(detail, tokens.type.meta, tokens.color.mute)
        }
    }
}

// A command that acts and leaves the sheet up, in a toolbar above what it acts on. Chips are for
// choosing between options; this is a verb.
@Composable
fun CommandButton(
    text: String,
    onClick: () -> Unit,
    icon: Icon? = null,
    primary: Boolean = false,
    danger: Boolean = false,
    label: String = text,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.md)
    val tone = when {
        primary -> tokens.color.onAccent
        danger -> tokens.color.blocked
        else -> tokens.color.text
    }
    Row(
        Modifier
            .background(if (primary) tokens.color.accent else tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .touchable(LANDSCAPE_TOUCH)
            .action(label, onClick, shape)
            .padding(horizontal = 13.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        if (icon != null) IconGlyph(icon, 14.dp, if (primary || danger) tone else tokens.color.dim)
        DisableSelection { KText(text, tokens.type.buttonSmall, tone) }
    }
}

@Composable
fun MenuDivider() {
    Divider(Modifier.fillMaxWidth().padding(vertical = 4.dp).height(1.dp))
}
