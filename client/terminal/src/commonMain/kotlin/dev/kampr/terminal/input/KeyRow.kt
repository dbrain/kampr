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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.edgeTop
import dev.kampr.shared.ui.gestureAction
import dev.kampr.shared.ui.group
import dev.kampr.shared.ui.named
import dev.kampr.shared.ui.pressable
import dev.kampr.terminal.PaneSession

// The bar's own inside margin, above the first row of caps and below the last. `safe.bottom` used
// to stand in for the second half, which held only while something was under the row: the moment
// the keyboard took the gesture handle — `KeyboardFloor` takes off whatever the keys already cover
// — the last row of caps sat flush on Gboard's first, with nothing between them.
fun keyRowPadding(compact: Boolean): Dp = if (compact) 6.dp else 10.dp

@Composable
fun PaneKeyRow(
    session: PaneSession,
    sink: InputSink,
    compact: Boolean,
    enabled: Boolean,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    val fn = session.latches.fn.active()
    val rows = KeyLayouts.rows(compact, fn)

    // No docking arithmetic here. The keyboard is paid for once, at the app root, by the only
    // surface that reaches the window's bottom edge. Reconciling `WindowInsets.ime` against a
    // slack derived from `containerSize` and `positionInWindow` looked right on an emulator with
    // no display cutout and left exactly the cutout's height as a gap on every phone that has one.
    //
    // What is left is a value, not a measurement: `bottom` is the gesture handle when this row is
    // what ends at the window, and zero when it is not — the pane screen's bottom navigation is
    // already holding the handle off, and it says so by taking the edge off what it holds. Which
    // is why this row does *not* pay on the pane screen, and why paying anyway put 46 dp of dead
    // strip between the last key and a navigation bar.
    val safe = LocalSafeArea.current

    Column(
        modifier
            .background(tokens.color.bar)
            .edgeTop()
            .group()
            .absolutePadding(
                left = 8.dp + safe.left,
                top = keyRowPadding(compact),
                right = 8.dp + safe.right,
                bottom = keyRowPadding(compact) + safe.bottom,
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
            // The caps drive themselves through `gestureAction` and a raw tap detector rather
            // than through `Modifier.action`, so they get nothing from the cursor `action` already
            // chains and hovered as a plain arrow on a desk.
            .then(
                if (!enabled) {
                    Modifier
                        .named("$spoken, unavailable on a read-only device")
                        .pressable(false)
                } else {
                    Modifier.pressable().gestureAction(
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
                },
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
            val which = riding ?: cap.latch
            which?.let(session.latches::tap)
            session.settleLatch(which)
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
            val hold = cap.hold
            if (hold != null) session.latches.tap(hold) else cap.latch?.let(session.latches::lock)
            session.settleLatch(hold ?: cap.latch)
        }
        CapKind.Keyboard -> session.toggleKeyboard()
        CapKind.Text -> {
            sink.press(cap.alternate ?: cap)
            session.reclaimKeyboard()
        }
    }
}

// Ctrl and alt are prefixes, and the key they take is nearly always a letter — the one thing this
// row does not carry. Arming one with the keyboard down leaves a chord that cannot be finished,
// and nothing on screen says so, because the cap lights up either way. So arming one is a request
// for the keyboard, the same as tapping the grid is.
//
// Not shift, which rides the arrows and tab the row already has, and not fn, which *is* the row.
// Nor clearing one: after that tap the latch is off, and turning a modifier off is the opposite of
// asking for something to modify.
private fun Latch.takesALetter(): Boolean = this == Latch.Ctrl || this == Latch.Alt

private fun PaneSession.settleLatch(latch: Latch?) {
    if (latch != null && latch.takesALetter() && latches[latch].active()) openKeyboard()
    reclaimKeyboard()
}
