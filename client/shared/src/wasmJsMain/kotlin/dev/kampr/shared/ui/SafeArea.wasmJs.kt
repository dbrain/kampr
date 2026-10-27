package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.js.ExperimentalWasmJsInterop

// A browser keyboard resizes the *visual* viewport and leaves the layout viewport — and so the
// canvas Compose measures itself against — exactly as tall as it was. The difference between the
// two is the same fact `WindowInsets.ime` carries on Android, and it is the only way to get it
// here: no Compose inset reports it on the web.
// Pushed, not polled: a frame awaited to read this is a `requestAnimationFrame` and a repaint of
// the whole scene, so polling it held every open tab at the display's refresh rate. There is one
// caller, `systemSafeArea` at the app's root, so one listener slot is enough.
@OptIn(ExperimentalWasmJsInterop::class)
private fun watchOsk(notify: (Double) -> Unit): Double = js(
    """
    (function () {
        var s = globalThis.__kamprOsk;
        if (!s) {
            s = { px: 0, notify: null };
            var measure = function () {
                var v = window.visualViewport;
                var px = v ? Math.max(0, window.innerHeight - v.height - v.offsetTop) : 0;
                if (px === s.px) return;
                s.px = px;
                if (s.notify) s.notify(px);
            };
            if (window.visualViewport) {
                window.visualViewport.addEventListener('resize', measure);
                window.visualViewport.addEventListener('scroll', measure);
            }
            measure();
            globalThis.__kamprOsk = s;
        }
        s.notify = notify;
        return s.px;
    })()
    """,
)

@Composable
internal actual fun imeInset(): Dp {
    var inset by remember { mutableStateOf(0.dp) }
    DisposableEffect(Unit) {
        inset = watchOsk { inset = it.dp }.dp
        onDispose { watchOsk {} }
    }
    return inset
}
