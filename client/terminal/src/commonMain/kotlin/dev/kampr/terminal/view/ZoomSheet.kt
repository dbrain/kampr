package dev.kampr.terminal.view

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.background
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.dp
import dev.kampr.shared.platform.LocalHardKeyboard
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.Divider
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.modal
import dev.kampr.shared.ui.gestureAction
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.touchable
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import kotlin.math.abs

// A key step is a wheel click, so the two agree; an arrow is the fine one, which is the whole
// reason a keyboard is worth binding here — a preset is a jump and a click is a click, and neither
// lands on a particular size.
private const val ZOOM_PER_KEY = 1.1f
private const val ZOOM_PER_ARROW = 1.02f

private const val SHEET_ROOM = 0.86f

@Composable
fun ZoomSheet(
    presets: ZoomPresets,
    zoom: Float,
    window: ColumnWindow,
    totalRows: Int,
    visibleRows: Int,
    historyNote: String?,
    remembered: Boolean,
    followCursor: Boolean,
    confirmRisky: Boolean,
    onZoom: (Float) -> Unit,
    onRemember: (Boolean) -> Unit,
    onFollow: (Boolean) -> Unit,
    onConfirmRisky: (Boolean) -> Unit,
    // Null on a read-only device and on a node that does not carry the op — the panel is hidden
    // rather than disabled, which is `ManageLayer`'s rule for everything `manage` can do.
    sizing: PaneSizing? = null,
    onResize: (Int, Int) -> Unit = { _, _ -> },
    onHoldSize: (Boolean) -> Unit = {},
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    // The hint said "pinch to adjust" on every platform, including the two where a pinch could not
    // reach the zoom at all: the grid's pinch needs two *pressed* pointers and a touchpad presses
    // nothing, and a mouse has no pinch to make. `LocalHardKeyboard` is a desk reading — on the web
    // it is `(hover: hover) and (pointer: fine)`, a mouse or a trackpad — so it names the wheel
    // there and keeps the finger's word everywhere else. Both are true at once on a trackpad, whose
    // pinch the browser delivers as exactly this ctrl+wheel.
    val adjust = if (LocalHardKeyboard.current) "pinch or ctrl+wheel" else "pinch to adjust"
    // One parent, the way `BottomSheet` builds it. The scrim covers the sheet as well as
    // the screen, so the two have to be siblings under a Box of their own — otherwise the
    // sheet's own controls are hit-tested against a scrim spread across the whole pane.
    Box(Modifier.fillMaxSize()) {
        // Both halves, the way `Scrim` does it: the semantic action is what TalkBack dispatches, and
        // the clickable is what catches the finger. With only the first, a tap fell straight through to
        // the grid underneath, which reads a tap as "raise the keyboard".
        Box(
            Modifier
                .fillMaxSize()
                .gestureAction("Close the zoom sheet", onDismiss)
                .clickable(remember { MutableInteractionSource() }, indication = null, onClick = onDismiss),
        )
        Column(
            modifier
                .fillMaxWidth()
                .modal(onDismiss)
                // Scoped to the open sheet, which is the only place these keys are free. While the
                // grid is live the terminal owns the keyboard: `FieldTextInput` consumes every ctrl
                // chord it recognises and drops the rest, and the wasm layer `preventDefault`s the
                // chord set straight into the shell.
                //
                // A *global* ctrl+= / ctrl+- was considered and refused, and not because the browser
                // would win it — #307 measured that a ctrl+wheel here is `preventDefault`ed and the
                // page never zooms, so it could be taken. It is refused because ctrl+- is already
                // the shell's: the wasm layer maps it to US (`ctrl+_`), which is readline's undo.
                // Ctrl+wheel needs no key at all and covers the mouse and the touchpad both, so the
                // global binding would cost a working shell chord and buy nothing.
                //
                // `onPreviewKeyEvent`, not `onKeyEvent`, because `modal`'s Escape handler is on the
                // bubble pass and a focused descendant would otherwise win — and returning false for
                // anything unrecognised is what leaves Escape to it.
                .onPreviewKeyEvent { event ->
                    if (event.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                    val next = when (event.key) {
                        Key.Equals, Key.Plus, Key.NumPadAdd -> zoom * ZOOM_PER_KEY
                        Key.Minus, Key.NumPadSubtract -> zoom / ZOOM_PER_KEY
                        Key.DirectionUp, Key.DirectionRight -> zoom * ZOOM_PER_ARROW
                        Key.DirectionDown, Key.DirectionLeft -> zoom / ZOOM_PER_ARROW
                        Key.One -> presets.fitWidth
                        Key.Two -> presets.readable
                        Key.Three -> presets.closeUp
                        else -> return@onPreviewKeyEvent false
                    }
                    onZoom(next)
                    true
                }
                .padding(horizontal = 12.dp, vertical = 11.dp),
        ) {
            Surface(Modifier.fillMaxWidth(), background = tokens.color.surface, radius = tokens.radii.lg) {
                // Measured *here*, inside the padding the caller stood the sheet off its chrome
                // with, and not off the window: sized against the window the sheet was 686 dp tall
                // in a 508 dp gap, so its top 189 dp — the Zoom line and the whole minimap — sat
                // behind the pane header, where no amount of scrolling could reach them. The
                // scroller was working the entire time; it was the viewport that was in the wrong
                // place.
                //
                // Short of the room rather than all of it, because the pane left showing is the
                // only thing left to tap: the scrim is under the header and the key row both, so a
                // sheet that filled its gap would have nowhere outside itself to dismiss it.
                BoxWithConstraints {
                    Column(
                        Modifier
                            .heightIn(max = maxHeight * SHEET_ROOM)
                            .verticalScroll(rememberScrollState())
                            .padding(horizontal = 15.dp, vertical = 14.dp),
                        verticalArrangement = Arrangement.spacedBy(13.dp),
                    ) {
                        Row(
                            Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            LabelText("Zoom", tokens.type.sectionLabel, tokens.color.mute)
                            KText("$adjust · ${zoomLabel(zoom)}", tokens.type.metaSmall, tokens.color.mute)
                        }

                        Surface(
                            Modifier.fillMaxWidth(),
                            background = tokens.color.surface2,
                            radius = tokens.radii.md,
                        ) {
                            Column(Modifier.padding(9.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                Minimap(
                                    window, totalRows, visibleRows,
                                    Modifier
                                        .fillMaxWidth()
                                        .height(74.dp)
                                        .named(
                                            "Showing columns ${window.firstCol + 1} to ${window.lastCol} " +
                                                "of ${window.cols}, and $visibleRows of $totalRows rows",
                                        ),
                                )
                                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                                    KText(
                                        "viewing col ${window.firstCol + 1}–${window.lastCol}, " +
                                            "row ${visibleRows.coerceAtMost(totalRows)} of $totalRows",
                                        tokens.type.metaSmall,
                                        tokens.color.mute,
                                    )
                                    KText("of ${window.cols} wide", tokens.type.metaSmall, tokens.color.mute)
                                }
                                // Only when there is a hole to own up to. A pane whose ring is intact says
                                // nothing here, which is what stops this becoming furniture.
                                if (historyNote != null) {
                                    KText(
                                        historyNote.replaceFirstChar(Char::uppercase),
                                        tokens.type.metaSmall,
                                        tokens.color.working,
                                        Modifier.named("The scrollback: $historyNote"),
                                        maxLines = 3,
                                    )
                                }
                            }
                        }

                        // The continuous one, above the three that are jumps. The presets stay: they
                        // are the sizes worth landing on exactly, and this is everything between them.
                        ZoomSlider(presets, zoom, onZoom)

                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(7.dp)) {
                            Preset("Fit width", presets.fitWidth, zoom, onZoom, Modifier.weight(1f))
                            Preset("Readable", presets.readable, zoom, onZoom, Modifier.weight(1f))
                            Preset("Close up", presets.closeUp, zoom, onZoom, Modifier.weight(1f))
                        }

                        if (sizing != null) {
                            Divider()
                            ResizePanel(sizing, onResize, onHoldSize)
                            Divider()
                        }

                        Toggle(
                            on = remembered,
                            title = "Remember for this pane",
                            detail = "Per device, stored on the node.",
                            onChange = onRemember,
                        )
                        Toggle(
                            on = followCursor,
                            title = "Follow the cursor",
                            detail = "Pans sideways to keep the caret in view.",
                            onChange = onFollow,
                        )
                        Toggle(
                            on = confirmRisky,
                            title = "Check destructive commands",
                            detail = "Holds Enter on rm -rf, sudo, force-push. Shell panes only.",
                            onChange = onConfirmRisky,
                        )
                        KText(
                            "Zoom is yours alone — the pane stays ${window.cols} columns for everyone. " +
                                "Pane size is not: it changes the pane for everybody looking at it.",
                            tokens.type.captionSmall,
                            tokens.color.mute,
                            maxLines = 2,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun Minimap(window: ColumnWindow, totalRows: Int, visibleRows: Int, modifier: Modifier) {
    val tokens = Kampr.tokens
    val ground = tokens.color.bg
    val bar = tokens.color.raise
    val accent = tokens.color.accent
    val wash = tokens.color.accentSoft
    Canvas(modifier.background(ground, RoundedCornerShape(tokens.radii.sm))) {
        val lines = 9
        val step = size.height / lines
        for (i in 0 until lines) {
            val width = size.width * (0.34f + 0.62f * ((i * 37 % 19) / 19f))
            drawRect(bar, Offset(6f, i * step + step * 0.35f), Size(width, 3f))
        }
        val cols = window.cols.coerceAtLeast(1)
        val left = size.width * (window.firstCol.toFloat() / cols)
        val width = size.width * ((window.lastCol - window.firstCol).toFloat() / cols)
        val height = size.height * (visibleRows.toFloat() / totalRows.coerceAtLeast(1)).coerceIn(0.1f, 1f)
        drawRoundRect(wash, Offset(left, size.height - height), Size(width, height), CornerRadius(3f, 3f))
        drawRoundRect(
            accent,
            Offset(left, size.height - height),
            Size(width, height),
            CornerRadius(3f, 3f),
            style = Stroke(width = 1.5f),
        )
    }
}

@Composable
private fun Preset(
    title: String,
    value: Float,
    zoom: Float,
    onZoom: (Float) -> Unit,
    modifier: Modifier,
) {
    val tokens = Kampr.tokens
    val active = abs(zoom - value) < 0.02f
    val shape = RoundedCornerShape(tokens.radii.sm)
    Column(
        modifier
            .background(if (active) tokens.color.accentSoft else tokens.color.raise, shape)
            .edge(if (active) BorderSpec(1.dp, tokens.color.accent) else tokens.card, shape)
            .touchable()
            .action("$title, ${zoomLabel(value)}", { onZoom(value) }, shape, selected = active)
            .padding(vertical = 10.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        KText(title, tokens.type.pill, if (active) tokens.color.accent else tokens.color.text)
        KText(zoomLabel(value), tokens.type.metaSmall, if (active) tokens.color.accent else tokens.color.mute)
    }
}

@Composable
internal fun Toggle(on: Boolean, title: String, detail: String, onChange: (Boolean) -> Unit) {
    val tokens = Kampr.tokens
    Row(
        Modifier
            .fillMaxWidth()
            .touchable()
            .action(
                "$title. $detail",
                { onChange(!on) },
                role = androidx.compose.ui.semantics.Role.Switch,
                selected = on,
                state = if (on) "on" else "off",
            ),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(11.dp),
    ) {
        Box(
            Modifier
                .width(40.dp)
                .height(23.dp)
                .background(
                    if (on) tokens.color.accent else tokens.color.raise,
                    RoundedCornerShape(tokens.radii.pill),
                )
                .padding(2.dp),
            contentAlignment = if (on) Alignment.CenterEnd else Alignment.CenterStart,
        ) {
            Box(
                Modifier
                    .size(19.dp)
                    .background(
                        if (on) tokens.color.onAccent else tokens.color.dim,
                        RoundedCornerShape(tokens.radii.pill),
                    ),
            )
        }
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            KText(title, tokens.type.cardTitle, tokens.color.text)
            KText(detail, tokens.type.captionSmall, tokens.color.mute)
        }
    }
}
