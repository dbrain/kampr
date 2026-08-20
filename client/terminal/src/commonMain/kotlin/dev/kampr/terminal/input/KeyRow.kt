package dev.kampr.terminal.input

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.edgeTop
import dev.kampr.terminal.PaneSession

@Composable
fun PaneKeyRow(
    session: PaneSession,
    sink: InputSink,
    compact: Boolean,
    enabled: Boolean,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val density = LocalDensity.current
    val fn = session.latches.fn.active()
    val rows = KeyLayouts.rows(compact, fn)

    // The on-screen keyboard's inset is measured from the bottom of the window, but this row sits
    // inside a container that may already stop short of it — a bottom navigation bar, for one. Pad
    // by the difference or the row floats exactly one nav bar above the keys, which is the gap.
    val windowHeight = LocalWindowInfo.current.containerSize.height.toFloat()
    var slack by remember { mutableFloatStateOf(0f) }
    val osk = with(density) { rememberOskInset().toPx() }
    val dock = with(density) { (osk - slack).coerceAtLeast(0f).toDp() }

    Column(
        modifier
            .onGloballyPositioned { coordinates ->
                slack = (windowHeight - coordinates.positionInWindow().y - coordinates.size.height)
                    .coerceAtLeast(0f)
            }
            .background(tokens.color.bar)
            .edgeTop()
            .padding(bottom = dock)
            .padding(start = 8.dp, top = if (compact) 6.dp else 10.dp, end = 8.dp),
        verticalArrangement = Arrangement.spacedBy(if (compact) 5.dp else 6.dp),
    ) {
        for (row in rows) {
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(if (compact) 5.dp else 6.dp),
            ) {
                for (cap in row) {
                    if (cap == null) Spacer(Modifier.width(if (compact) 14.dp else 10.dp))
                    else Cap(cap, session, sink, compact, enabled)
                }
            }
        }
    }
}

@Composable
private fun RowScope.Cap(
    cap: KeyCap,
    session: PaneSession,
    sink: InputSink,
    compact: Boolean,
    enabled: Boolean,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    val held = cap.hold?.let { session.latches[it] } ?: LatchState.Off
    val state = when {
        held.active() -> held
        else -> cap.latch?.let { session.latches[it] } ?: LatchState.Off
    }
    val background = when {
        !enabled -> tokens.color.surface
        state == LatchState.Locked -> tokens.color.accentHi
        state == LatchState.Armed -> tokens.color.accent
        cap.kind == CapKind.Keyboard && session.keyboardOpen -> tokens.color.accentSoft
        else -> tokens.color.raise
    }
    val ink = when {
        !enabled -> tokens.color.mute
        state.active() -> tokens.color.onAccent
        else -> tokens.color.text
    }

    Box(
        Modifier
            .weight(1f)
            .background(background, shape)
            .edge(tokens.card, shape)
            .pointerInput(cap, enabled) {
                if (!enabled) return@pointerInput
                detectTapGestures(
                    onTap = { press(cap, session, sink) },
                    onLongPress = { hold(cap, session, sink) },
                )
            }
            .defaultMinSize(minHeight = 44.dp)
            .padding(vertical = if (compact) 7.dp else 12.dp),
        contentAlignment = Alignment.Center,
    ) {
        val label = when {
            cap.hold != null && held.active() -> holdLabel(cap.hold)
            else -> cap.label
        }
        KText(label, if (cap.symbol) tokens.type.badge else tokens.type.key, ink)
    }
}

private fun holdLabel(latch: Latch): String = when (latch) {
    Latch.Shift -> "shift"
    Latch.Fn -> "fn"
    Latch.Ctrl -> "ctrl"
    Latch.Alt -> "alt"
}

private fun press(cap: KeyCap, session: PaneSession, sink: InputSink) {
    when (cap.kind) {
        CapKind.Latch -> {
            val riding = cap.hold?.takeIf { session.latches[it].active() }
            if (riding != null) session.latches.tap(riding) else cap.latch?.let(session.latches::tap)
        }
        CapKind.Keyboard -> session.closeKeyboard()
        CapKind.Text -> sink.press(cap)
    }
}

private fun hold(cap: KeyCap, session: PaneSession, sink: InputSink) {
    when (cap.kind) {
        CapKind.Latch -> cap.hold?.let(session.latches::tap) ?: cap.latch?.let(session.latches::lock)
        CapKind.Keyboard -> session.closeKeyboard()
        CapKind.Text -> sink.press(cap.alternate ?: cap)
    }
}
