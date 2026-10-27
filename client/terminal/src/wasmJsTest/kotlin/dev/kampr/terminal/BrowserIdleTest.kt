package dev.kampr.terminal

import androidx.compose.runtime.AbstractApplier
import androidx.compose.runtime.BroadcastFrameClock
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Composition
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Recomposer
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.platform.ViewConfiguration
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.Latches
import dev.kampr.terminal.input.PaneTextInput
import dev.kampr.terminal.input.focusInput
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlin.test.Test
import kotlin.test.assertFalse

private object Inert : AbstractApplier<Any?>(null) {
    override fun insertTopDown(index: Int, instance: Any?) = Unit
    override fun insertBottomUp(index: Int, instance: Any?) = Unit
    override fun remove(index: Int, count: Int) = Unit
    override fun move(from: Int, to: Int, count: Int) = Unit
    override fun onClear() = Unit
}

private object Untouched : ViewConfiguration {
    override val longPressTimeoutMillis = 0L
    override val doubleTapTimeoutMillis = 0L
    override val doubleTapMinTimeMillis = 0L
    override val touchSlop = 0f
}

// Whether anything composed is still waiting on a frame once it has settled. In a browser every
// frame anything awaits is a `requestAnimationFrame` and a full repaint of the scene, so a loop
// that awaits one per turn holds the tab at the display's refresh rate for as long as it is open.
private fun TestScope.asksForFramesAtRest(content: @Composable () -> Unit): Boolean {
    val clock = BroadcastFrameClock()
    val recomposer = Recomposer(coroutineContext + clock)
    val running = launch(clock) { recomposer.runRecomposeAndApplyChanges() }
    val composition = Composition(Inert, recomposer)
    composition.setContent {
        CompositionLocalProvider(
            LocalDensity provides Density(1f),
            LocalLayoutDirection provides LayoutDirection.Ltr,
            LocalViewConfiguration provides Untouched,
            content = content,
        )
    }
    repeat(SETTLE_FRAMES) {
        runCurrent()
        clock.sendFrame(it * FRAME_NANOS)
    }
    runCurrent()
    val asking = clock.hasAwaiters
    composition.dispose()
    recomposer.cancel()
    running.cancel()
    return asking
}

private const val SETTLE_FRAMES = 10
private const val FRAME_NANOS = 16_000_000L

// The report: a Kampr tab held half a core with nothing on it moving. The pane's claim on the
// keyboard was renewed by awaiting a frame every frame, and the keyboard inset had the same shape.
class BrowserIdleTest {
    @Test
    fun aPaneHoldingTheKeyboardAsksForNoFramesAtRest() = runTest {
        val session = PaneSession(BROWSER_PANE)
        val sink = InputSink(BROWSER_PANE, Hush, Latches())
        val asking = asksForFramesAtRest {
            PaneTextInput(session, sink, enabled = true, onChord = {}, modifier = Modifier)
        }
        focusInput(false)
        assertFalse(asking, "a pane holding the keyboard renews its claim every frame")
    }
}
