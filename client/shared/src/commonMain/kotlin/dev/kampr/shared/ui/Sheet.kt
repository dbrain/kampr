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
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.foundation.focusable
import androidx.compose.ui.focus.focusRequester
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.withFrameNanos
import androidx.compose.runtime.remember
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr

private val SHEET_MAX_WIDTH = 420.dp
// A 844x390 landscape viewport is wide and short, so the sheet spends the width it has rather
// than scrolling a phone-shaped column through a letterbox.
private val SHEET_WIDE_WIDTH = 720.dp
private const val SCRIM_ALPHA = 0.72f

// The requester is refused until the sheet is placed and the focus owner is live, and how many
// frames that takes is not something a caller can know.
private const val FOCUS_FRAMES = 8

// The scrim is the theme's own ground at partial opacity rather than a colour of its own, so it
// stays right on a black brutalist ground and on a light one.
@Composable
fun Scrim(onDismiss: () -> Unit, label: String = "Close", modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    Box(
        modifier
            .fillMaxSize()
            .background(tokens.color.bg.copy(alpha = SCRIM_ALPHA))
            .gestureAction(label, onClick = onDismiss)
            .clickable(remember { MutableInteractionSource() }, indication = null, onClick = onDismiss)
    )
}

@Composable
fun selectedEdge(): BorderSpec =
    BorderSpec(if (Kampr.tokens.card.visible) Kampr.tokens.card.width else Kampr.tokens.chrome.width, Kampr.tokens.color.accent)

// Escape closes it and focus moves into it on open, so the sheet is operable from a keyboard.
// What holds the reader inside it is not this composable: the screen underneath drops out of the
// semantics tree while a sheet is up (see `AppScaffold`), which is what a trap buys and this is
// how Kampr buys it without turning every sheet into a platform dialog.
// Everything modal wears this: Escape dismisses it, focus moves into it on open, and a reader
// traverses it before the screen it is covering. The screen behind stops being readable at all —
// see `behindSheet` in KamprApp — which is what a focus trap buys without a platform dialog.
@Composable
fun Modifier.modal(onDismiss: () -> Unit): Modifier {
    val focus = remember { FocusRequester() }
    // Late on purpose: the requester is refused until the sheet is placed and the focus owner is
    // live, and a sheet that never takes focus is one whose Escape goes to the screen behind.
    LaunchedEffect(focus) {
        repeat(FOCUS_FRAMES) {
            withFrameNanos { }
            if (runCatching { focus.requestFocus() }.getOrDefault(false)) return@LaunchedEffect
        }
    }
    return this
        .focusRequester(focus)
        .focusable()
        .readingOrder(-1f)
        .escapes(onDismiss)
}

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
            val wanted = if (breakpoint == Breakpoint.Portrait) SHEET_MAX_WIDTH else SHEET_WIDE_WIDTH
            val width = if (maxWidth < wanted) maxWidth else wanted
            val tall = maxHeight * (if (breakpoint == Breakpoint.Portrait) 0.92f else 0.94f)
            val shape = RoundedCornerShape(tokens.radii.lg)
            Column(
                Modifier
                    .width(width)
                    .heightIn(max = tall)
                    .background(tokens.color.bar, shape)
                    .modal(onDismiss)
                    .edge(tokens.chrome, shape),
                content = content,
            )
        }
    }
}

// Escape is the one key every sheet and dialog owes a keyboard user, and none of them had it.
private fun Modifier.escapes(onDismiss: () -> Unit): Modifier = onKeyEvent { event ->
    if (event.type == KeyEventType.KeyDown && event.key == Key.Escape) {
        onDismiss()
        true
    } else {
        false
    }
}

// A 390 dp-tall landscape viewport cannot afford a 26 px title and two rows of chrome, so the
// header collapses onto one line rather than eating a third of the sheet.
@Composable
fun SheetHeader(
    title: String,
    subtitle: String?,
    onBack: (() -> Unit)?,
    onClose: () -> Unit,
    compact: Boolean = false,
) {
    val tokens = Kampr.tokens
    Column {
        Row(
            Modifier
                .fillMaxWidth()
                .padding(
                    start = 20.dp,
                    top = if (compact) 12.dp else 18.dp,
                    end = 20.dp,
                    bottom = if (compact) 10.dp else 12.dp,
                ),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (onBack != null) {
                BackAction("Back", onBack, target = if (compact) LANDSCAPE_TOUCH else TOUCH)
            }
            KText(title, if (compact) tokens.type.paneTitle else tokens.type.screenTitle, tokens.color.text)
            if (compact && subtitle != null) {
                LabelText(subtitle, tokens.type.micro, tokens.color.mute, Modifier.weight(1f))
            } else {
                Box(Modifier.weight(1f))
            }
            CloseAction("Close $title", onClose, target = if (compact) LANDSCAPE_TOUCH else TOUCH)
        }
        if (!compact && subtitle != null) {
            Box(Modifier.padding(start = 20.dp, end = 20.dp, bottom = 9.dp)) {
                LabelText(subtitle, tokens.type.micro, tokens.color.mute)
            }
        }
    }
}

@Composable
fun SheetSection(label: String, compact: Boolean = false) {
    val tokens = Kampr.tokens
    Box(Modifier.padding(start = 20.dp, top = if (compact) 12.dp else 20.dp, end = 20.dp, bottom = 9.dp)) {
        LabelText(label, tokens.type.micro, tokens.color.mute)
    }
}

@Composable
fun SheetCard(
    icon: Icon?,
    iconTint: Color?,
    title: String,
    subtitle: String,
    subtitleMono: Boolean = false,
    selected: Boolean = false,
    compact: Boolean = false,
    onClick: (() -> Unit)? = null,
    label: String? = null,
    trailing: @Composable (() -> Unit)? = null,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.lg)
    Row(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.surface, shape)
            .edge(if (selected) selectedEdge() else tokens.card, shape)
            .let {
                if (onClick == null) it.named(listOf(title, subtitle).joinToString(", "))
                else it.touchable().action(label ?: "$title, $subtitle", onClick, shape, selected = selected)
            }
            .padding(horizontal = 15.dp, vertical = if (compact) 9.dp else 13.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (icon != null) {
            if (compact) Badge(28.dp, 14.dp, icon, iconTint ?: tokens.color.accent)
            else Badge(36.dp, 17.dp, icon, iconTint ?: tokens.color.accent)
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
fun Chip(
    text: String,
    selected: Boolean,
    onClick: () -> Unit,
    quiet: Boolean = false,
    label: String? = null,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Box(
        Modifier
            .background(if (selected) tokens.color.accent else tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .touchable(LANDSCAPE_TOUCH)
            .action(label ?: text, onClick, shape, selected = selected)
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
