package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.snapshotFlow
import kotlinx.browser.window
import org.w3c.dom.PopStateEvent
import org.w3c.dom.events.Event
import kotlin.js.ExperimentalWasmJsInterop

private const val ARM = "kampr-back"

@OptIn(ExperimentalWasmJsInterop::class)
private fun isArm(state: JsAny?): Boolean = (state as? JsString)?.toString() == ARM

// No URL is passed, so the address bar keeps the deep link the page was opened on.
@OptIn(ExperimentalWasmJsInterop::class)
private object WindowHistory : BrowserHistory {
    override fun push() = window.history.pushState(ARM.toJsString(), "")

    override fun back() = window.history.back()
}

// navigationevent's `BrowserInput` mirrors a stack of `NavigationEventInfo` into history, one
// entry per destination; the app's back is one handler with no such stack, so it would arm nothing.
@OptIn(ExperimentalWasmJsInterop::class)
@Composable
internal actual fun BrowserBack(wanted: () -> Boolean, back: () -> Unit) {
    val latestWanted by rememberUpdatedState(wanted)
    val latestBack by rememberUpdatedState(back)
    val arm = remember {
        BackArm(WindowHistory, isArm(window.history.state), { latestWanted() }, { latestBack() })
    }
    DisposableEffect(arm) {
        val listener: (Event) -> Unit = { arm.popped(isArm((it as PopStateEvent).state)) }
        window.addEventListener("popstate", listener)
        onDispose { window.removeEventListener("popstate", listener) }
    }
    LaunchedEffect(arm) { snapshotFlow { latestWanted() }.collect { arm.sync() } }
}
