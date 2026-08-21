package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.js.ExperimentalWasmJsInterop

// A browser keyboard resizes the *visual* viewport and leaves the layout viewport — and so the
// canvas Compose measures itself against — exactly as tall as it was. The difference between the
// two is the same fact `WindowInsets.ime` carries on Android, and it is the only way to get it
// here: no Compose inset reports it on the web.
@OptIn(ExperimentalWasmJsInterop::class)
private fun oskCssPx(): Double = js(
    """
    (function () {
        var s = globalThis.__kamprOsk;
        if (!s) {
            s = { px: 0 };
            var measure = function () {
                var v = window.visualViewport;
                s.px = v ? Math.max(0, window.innerHeight - v.height - v.offsetTop) : 0;
            };
            if (window.visualViewport) {
                window.visualViewport.addEventListener('resize', measure);
                window.visualViewport.addEventListener('scroll', measure);
            }
            measure();
            globalThis.__kamprOsk = s;
        }
        return s.px;
    })()
    """
)

// Polled rather than pushed: the listeners above fire on a thread of the browser's choosing and
// there is no Compose-side channel to hand the value to. A frame at a time, and state is only
// written when the number moves, so a shut keyboard costs one read per frame and no recomposition.
@Composable
internal actual fun imeInset(): Dp {
    var inset by remember { mutableStateOf(0.dp) }
    LaunchedEffect(Unit) {
        while (true) {
            withFrameNanos { }
            val next = oskCssPx().dp
            if (next != inset) inset = next
        }
    }
    return inset
}
