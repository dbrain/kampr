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
import androidx.compose.foundation.layout.absolutePadding
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
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.edgeTop
import dev.kampr.shared.ui.gestureAction
import dev.kampr.shared.ui.group
import dev.kampr.shared.ui.named
import dev.kampr.terminal.PaneSession
import kotlin.math.max

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
    //
    // The gesture handle is the same kind of fact as the keyboard — something the system draws
    // across the bottom of the window — so it goes through the same subtraction rather than a
    // padding of its own. Whichever is taller wins: the keyboard's inset already covers the
    // handle, and on the pane screen the bottom navigation covers it too, so this adds nothing
    // there and everything in the mosaic switcher, which has no navigation under it at all.
    val safe = LocalSafeArea.current
    val windowHeight = LocalWindowInfo.current.containerSize.height.toFloat()
    var slack by remember { mutableFloatStateOf(0f) }
    val osk = with(density) { rememberOskInset().toPx() }
    val floor = with(density) { max(osk, safe.bottom.toPx()) }
    val dock = with(density) { (floor - slack).coerceAtLeast(0f).toDp() }

    Column(
        modifier
            .onGloballyPositioned { coordinates ->
                slack = (windowHeight - coordinates.positionInWindow().y - coordinates.size.height)
                    .coerceAtLeast(0f)
            }
            .background(tokens.color.bar)
            .edgeTop()
            .group()
            .padding(bottom = dock)
            .absolutePadding(
                left = 8.dp + safe.left,
                top = if (compact) 6.dp else 10.dp,
                right = 8.dp + safe.right,
            ),
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

    val label = when {
        cap.hold != null && held.active() -> holdLabel(cap.hold)
        else -> cap.label
    }
    // A pointerInput block never sees TalkBack's double tap, so the caps carried a gesture and no
    // action: reachable, named nothing, and impossible to press. The names and the two actions are
    // the same press and hold the finger gets.
    val spoken = spokenKey(label) + if (cap.kind == CapKind.Text) " key" else ""
    val alternate = cap.alternate?.let { spokenKey(it.label) }
    Box(
        Modifier
            .weight(1f)
            .background(background, shape)
            .edge(tokens.card, shape)
            .then(
                if (!enabled) Modifier.named("$spoken, unavailable on a read-only device")
                else Modifier.gestureAction(
                    label = spoken,
                    onClick = { capPress(cap, session, sink) },
                    onLongClick = { capHold(cap, session, sink) },
                    state = when {
                        state == LatchState.Locked -> "locked"
                        state == LatchState.Armed -> "armed for the next key"
                        cap.kind == CapKind.Keyboard && session.keyboardOpen -> "keyboard showing"
                        else -> null
                    },
                    longLabel = when {
                        cap.hold != null -> holdLabel(cap.hold).replaceFirstChar(Char::uppercase)
                        alternate != null -> alternate
                        cap.kind == CapKind.Latch -> "Lock"
                        else -> null
                    },
                )
            )
            .pointerInput(cap, enabled) {
                if (!enabled) return@pointerInput
                detectTapGestures(
                    onTap = { capPress(cap, session, sink) },
                    onLongPress = { capHold(cap, session, sink) },
                )
            }
            .defaultMinSize(minHeight = 44.dp)
            .padding(vertical = if (compact) 7.dp else 12.dp),
        contentAlignment = Alignment.Center,
    ) {
        KText(label, if (cap.symbol) tokens.type.badge else tokens.type.key, ink)
    }
}

private fun holdLabel(latch: Latch): String = when (latch) {
    Latch.Shift -> "shift"
    Latch.Fn -> "fn"
    Latch.Ctrl -> "ctrl"
    Latch.Alt -> "alt"
}

internal fun capPress(cap: KeyCap, session: PaneSession, sink: InputSink) {
    when (cap.kind) {
        CapKind.Latch -> {
            val riding = cap.hold?.takeIf { session.latches[it].active() }
            if (riding != null) session.latches.tap(riding) else cap.latch?.let(session.latches::tap)
            session.reclaimKeyboard()
        }
        CapKind.Keyboard -> session.toggleKeyboard()
        CapKind.Text -> {
            sink.press(cap)
            session.reclaimKeyboard()
        }
    }
}

internal fun capHold(cap: KeyCap, session: PaneSession, sink: InputSink) {
    when (cap.kind) {
        CapKind.Latch -> {
            cap.hold?.let(session.latches::tap) ?: cap.latch?.let(session.latches::lock)
            session.reclaimKeyboard()
        }
        CapKind.Keyboard -> session.toggleKeyboard()
        CapKind.Text -> {
            sink.press(cap.alternate ?: cap)
            session.reclaimKeyboard()
        }
    }
}
