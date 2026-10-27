package dev.kampr.shared

import androidx.compose.runtime.AbstractApplier
import androidx.compose.runtime.BroadcastFrameClock
import androidx.compose.runtime.Composition
import androidx.compose.runtime.Recomposer
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.ui.imeInset
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse

private object Inert : AbstractApplier<Any?>(null) {
    override fun insertTopDown(index: Int, instance: Any?) = Unit
    override fun insertBottomUp(index: Int, instance: Any?) = Unit
    override fun remove(index: Int, count: Int) = Unit
    override fun move(from: Int, to: Int, count: Int) = Unit
    override fun onClear() = Unit
}

// A keyboard 300 px tall, the way a phone browser reports one: the layout viewport keeps its
// height and the visual viewport loses it.
@OptIn(ExperimentalWasmJsInterop::class)
private fun raiseKeyboard(px: Int) {
    js(
        """
        (function () {
            if (!globalThis.__kamprRealInnerHeight) {
                globalThis.__kamprRealInnerHeight = Object.getOwnPropertyDescriptor(window, 'innerHeight');
            }
            Object.defineProperty(window, 'innerHeight', { value: window.visualViewport.height + px, configurable: true });
            window.visualViewport.dispatchEvent(new Event('resize'));
        })()
        """,
    )
}

@OptIn(ExperimentalWasmJsInterop::class)
private fun lowerKeyboard() {
    js(
        """
        (function () {
            var real = globalThis.__kamprRealInnerHeight;
            if (!real) return;
            Object.defineProperty(window, 'innerHeight', real);
            globalThis.__kamprRealInnerHeight = null;
            window.visualViewport.dispatchEvent(new Event('resize'));
        })()
        """,
    )
}

// The report: a Kampr tab held half a core with nothing on it moving. In a browser every frame
// anything awaits is a `requestAnimationFrame` and a repaint of the whole scene, and the keyboard
// inset awaited one every frame just to read a number that moves only when a keyboard does.
class BrowserIdleTest {
    @Test
    fun theKeyboardInsetAsksForNoFramesAndStillFollowsTheKeyboard() = runTest {
        val clock = BroadcastFrameClock()
        val recomposer = Recomposer(coroutineContext + clock)
        val running = launch(clock) { recomposer.runRecomposeAndApplyChanges() }
        var inset: Dp = (-1).dp
        val composition = Composition(Inert, recomposer)
        composition.setContent { inset = imeInset() }
        fun settle() = repeat(SETTLE_FRAMES) {
            Snapshot.sendApplyNotifications()
            runCurrent()
            clock.sendFrame(it * FRAME_NANOS)
            runCurrent()
        }
        try {
            settle()
            assertFalse(clock.hasAwaiters, "the keyboard inset polls the browser every frame")
            raiseKeyboard(300)
            settle()
            assertEquals(300.dp, inset, "the inset never noticed the keyboard come up")
            lowerKeyboard()
            settle()
            assertEquals(0.dp, inset, "the inset never noticed the keyboard go away")
        } finally {
            lowerKeyboard()
            composition.dispose()
            recomposer.cancel()
            running.cancel()
        }
    }
}

private const val SETTLE_FRAMES = 10
private const val FRAME_NANOS = 16_000_000L
