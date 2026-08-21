package dev.kampr.terminal.view

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
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
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.Surface
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.modal
import dev.kampr.shared.ui.gestureAction
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.touchable
import kotlin.math.abs

private fun format(value: Float): String {
    val tenths = (value * 10f + 0.5f).toInt()
    return "${tenths / 10}.${tenths % 10}×"
}

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
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
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
                .padding(horizontal = 12.dp, vertical = 11.dp),
        ) {
            Surface(Modifier.fillMaxWidth(), background = tokens.color.surface, radius = tokens.radii.lg) {
                Column(
                    Modifier.padding(horizontal = 15.dp, vertical = 14.dp),
                    verticalArrangement = Arrangement.spacedBy(13.dp),
                ) {
                    Row(
                        Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        LabelText("Zoom", tokens.type.sectionLabel, tokens.color.mute)
                        KText("pinch to adjust · ${format(zoom)}", tokens.type.metaSmall, tokens.color.mute)
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

                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(7.dp)) {
                        Preset("Fit width", presets.fitWidth, zoom, onZoom, Modifier.weight(1f))
                        Preset("Readable", presets.readable, zoom, onZoom, Modifier.weight(1f))
                        Preset("Close up", presets.closeUp, zoom, onZoom, Modifier.weight(1f))
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
                        "Zoom is yours alone. The pane stays ${window.cols} columns for everyone — " +
                            "Kampr never resizes a session.",
                        tokens.type.captionSmall,
                        tokens.color.mute,
                        maxLines = 2,
                    )
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
            .action("$title, ${format(value)}", { onZoom(value) }, shape, selected = active)
            .padding(vertical = 10.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        KText(title, tokens.type.pill, if (active) tokens.color.accent else tokens.color.text)
        KText(format(value), tokens.type.metaSmall, if (active) tokens.color.accent else tokens.color.mute)
    }
}

@Composable
private fun Toggle(on: Boolean, title: String, detail: String, onChange: (Boolean) -> Unit) {
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
