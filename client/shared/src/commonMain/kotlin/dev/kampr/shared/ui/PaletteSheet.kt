package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr

// The terminal's own input never sees Ctrl+K on the web — it is a DOM element outside Compose and
// hands the chord back through `PaneChord.Palette` — so this is how it reaches the palette.
val LocalPalette = staticCompositionLocalOf<() -> Unit> { {} }

private val DIGITS = listOf(Key.One, Key.Two, Key.Three, Key.Four, Key.Five, Key.Six, Key.Seven, Key.Eight, Key.Nine)

fun isPaletteChord(event: KeyEvent): Boolean =
    event.type == KeyEventType.KeyDown && event.key == Key.K && !event.isShiftPressed && !event.isAltPressed &&
        (event.isCtrlPressed != event.isMetaPressed)

@Composable
fun PaletteHost(
    items: () -> List<PaletteItem>,
    onPick: (PaletteTarget) -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    Box(
        modifier.onPreviewKeyEvent { event ->
            if (!isPaletteChord(event)) return@onPreviewKeyEvent false
            open = !open
            true
        },
    ) {
        CompositionLocalProvider(
            LocalPalette provides { open = true },
            LocalCovered provides (open || LocalCovered.current),
        ) { content() }
        if (open) {
            CommandPalette(
                items = remember { items() },
                onPick = { open = false; onPick(it) },
                onDismiss = { open = false },
            )
        }
    }
}

private const val SEARCH_LABEL = "Search every pane and place"

// Shaped like the field it opens, so it reads as search on a phone where nobody presses ctrl+K.
@Composable
fun PaletteBar(hint: String?, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val open = LocalPalette.current
    val shape = RoundedCornerShape(tokens.radii.sm)
    Row(
        modifier
            .fillMaxWidth()
            .background(tokens.color.surface2, shape)
            .edge(tokens.card, shape)
            .touchable(LANDSCAPE_TOUCH)
            .action(SEARCH_LABEL, open, shape)
            .padding(horizontal = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        IconGlyph(KamprIcons.search, 13.dp, tokens.color.mute)
        KText("Search", tokens.type.captionSmall, tokens.color.mute, Modifier.weight(1f))
        if (hint != null) KText(hint, tokens.type.micro, tokens.color.mute)
    }
}

@Composable
fun PaletteAction(target: Dp = TOUCH, modifier: Modifier = Modifier) {
    GlyphAction(KamprIcons.search, SEARCH_LABEL, Kampr.tokens.color.dim, target, modifier, onClick = LocalPalette.current)
}

@Composable
fun CommandPalette(items: List<PaletteItem>, onPick: (PaletteTarget) -> Unit, onDismiss: () -> Unit) {
    val tokens = Kampr.tokens
    var query by remember { mutableStateOf("") }
    var selected by remember { mutableStateOf(0) }
    val shown = remember(items, query) { paletteSearch(items, query).take(PALETTE_SHOWN) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(focus) {
        repeat(FOCUS_FRAMES) {
            withFrameNanos { }
            if (runCatching { focus.requestFocus() }.getOrDefault(false)) return@LaunchedEffect
        }
    }
    fun pick(index: Int) {
        shown.getOrNull(index)?.let { onPick(it.target) }
    }

    BoxWithConstraints(Modifier.fillMaxSize()) {
        Scrim(onDismiss, label = "Close search")
        Column(
            Modifier
                .align(Alignment.TopCenter)
                .padding(top = 64.dp, start = 16.dp, end = 16.dp)
                .widthIn(max = 560.dp)
                .width(maxWidth)
                .background(tokens.color.surface, RoundedCornerShape(tokens.radii.lg))
                .edge(tokens.card, RoundedCornerShape(tokens.radii.lg))
                .readingOrder(-1f)
                .onPreviewKeyEvent { event ->
                    if (event.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                    val bare = !event.isCtrlPressed && !event.isMetaPressed && !event.isAltPressed
                    val digit = DIGITS.indexOf(event.key)
                    when {
                        event.key == Key.Escape -> onDismiss()
                        event.key == Key.Enter || event.key == Key.NumPadEnter -> pick(selected)
                        event.key == Key.DirectionDown -> selected = (selected + 1).coerceAtMost(shown.lastIndex.coerceAtLeast(0))
                        event.key == Key.DirectionUp -> selected = (selected - 1).coerceAtLeast(0)
                        digit >= 0 && bare && !event.isShiftPressed -> pick(digit)
                        else -> return@onPreviewKeyEvent false
                    }
                    true
                }
                .padding(10.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            KField(
                hint = "host, session, path, agent…",
                text = query,
                modifier = Modifier.focusRequester(focus),
                style = tokens.type.body,
                label = "Search panes and places",
                onSubmit = { pick(selected) },
                onText = {
                    query = it
                    selected = 0
                },
            )
            if (shown.isEmpty()) {
                KText("nothing matches", tokens.type.caption, tokens.color.mute, Modifier.padding(8.dp))
            }
            shown.forEachIndexed { index, item ->
                PaletteRow(index, item, index == selected) { onPick(item.target) }
            }
            KText(
                "1–9 to jump · ↑↓ enter · esc",
                tokens.type.captionSmall,
                tokens.color.mute,
                Modifier.padding(horizontal = 8.dp),
            )
        }
    }
}

@Composable
private fun PaletteRow(index: Int, item: PaletteItem, selected: Boolean, onClick: () -> Unit) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Row(
        Modifier
            .fillMaxWidth()
            .let { if (selected) it.background(tokens.color.raise, shape) else it }
            .action("${index + 1}: ${item.title}, ${item.detail}", onClick, shape, role = Role.Button, selected = selected)
            .padding(horizontal = 8.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        KText("${index + 1}", tokens.type.meta, if (selected) tokens.color.accent else tokens.color.mute)
        Box(Modifier.width(8.dp), contentAlignment = Alignment.Center) {
            item.status?.let { StatusMark(it, 7.dp) }
        }
        Column(Modifier.weight(1f)) {
            KText(item.title, tokens.type.cardTitle, tokens.color.text, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
            if (item.detail.isNotEmpty()) {
                KText(item.detail, tokens.type.meta, tokens.color.mute, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
            }
        }
    }
}
