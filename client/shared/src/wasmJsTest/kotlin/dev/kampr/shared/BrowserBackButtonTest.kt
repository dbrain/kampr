package dev.kampr.shared

import androidx.compose.runtime.AbstractApplier
import androidx.compose.runtime.BroadcastFrameClock
import androidx.compose.runtime.Composition
import androidx.compose.runtime.Recomposer
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import dev.kampr.shared.ui.BrowserBack
import kotlinx.browser.window
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.w3c.dom.events.Event
import kotlin.coroutines.resume
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.test.Test
import kotlin.test.assertEquals

private object NoNodes : AbstractApplier<Any?>(null) {
    override fun insertTopDown(index: Int, instance: Any?) = Unit
    override fun insertBottomUp(index: Int, instance: Any?) = Unit
    override fun remove(index: Int, count: Int) = Unit
    override fun move(from: Int, to: Int, count: Int) = Unit
    override fun onClear() = Unit
}

private suspend fun nextPopstate() = suspendCancellableCoroutine { done ->
    lateinit var once: (Event) -> Unit
    once = {
        window.removeEventListener("popstate", once)
        done.resume(Unit)
    }
    window.addEventListener("popstate", once)
}

@OptIn(ExperimentalWasmJsInterop::class)
private fun armed(): Boolean = (window.history.state as? JsString)?.toString() == "kampr-back"

// The report: back in a browser tab left Kampr with a file open. Driven through the page's real
// History, because a popstate arriving after the traversal rather than during it is the whole point.
@OptIn(ExperimentalCoroutinesApi::class)
class BrowserBackButtonTest {
    @Test
    fun theBrowsersBackButtonDoesTheAppsBackAndLeavesOnlyFromTheRoot() = runTest {
        val clock = BroadcastFrameClock()
        val recomposer = Recomposer(coroutineContext + clock)
        val running = launch(clock) { recomposer.runRecomposeAndApplyChanges() }
        var depth by mutableIntStateOf(0)
        var backs = 0
        val composition = Composition(NoNodes, recomposer)
        composition.setContent {
            BrowserBack(wanted = { depth > 0 }, back = {
                backs++
                depth--
            })
        }
        fun TestScope.settle() = repeat(SETTLE_FRAMES) {
            Snapshot.sendApplyNotifications()
            runCurrent()
            clock.sendFrame(it * FRAME_NANOS)
            runCurrent()
        }
        try {
            settle()
            val base = window.history.length
            assertEquals(false, armed(), "nothing to go back to, so nothing pushed")

            depth = 2
            settle()
            assertEquals(base + 1, window.history.length)
            assertEquals(true, armed())

            window.history.back()
            nextPopstate()
            settle()
            assertEquals(1, depth, "the browser's back did not reach the app")
            assertEquals(true, armed(), "still somewhere to go, so the next back has to be caught too")

            window.history.back()
            nextPopstate()
            settle()
            assertEquals(0, depth)
            assertEquals(false, armed(), "at the root the next browser back must leave")

            depth = 1
            settle()
            assertEquals(true, armed())
            depth = 0
            val stepped = launch { nextPopstate() }
            settle()
            stepped.join()
            settle()
            assertEquals(false, armed(), "an in-app back to the root has to give the arm back")
            assertEquals(2, backs, "giving the arm back is not itself a back")
        } finally {
            composition.dispose()
            recomposer.cancel()
            running.cancel()
        }
    }
}

private const val SETTLE_FRAMES = 10
private const val FRAME_NANOS = 16_000_000L
