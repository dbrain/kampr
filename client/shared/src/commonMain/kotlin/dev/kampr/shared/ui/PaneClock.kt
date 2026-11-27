package dev.kampr.shared.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import kotlinx.coroutines.delay

// One clock for everything a pane counts. `now` moves every second for the counters a reader
// watches (a retry, a reset, a running task); `minute` moves once a minute for the transcript's
// stamps, which are re-bucketed on every read and are not worth a frame a second.
//
// `read` is a member and the class is @Stable: on Kotlin/Wasm a top-level initialiser in this file
// would move the class's Compose `$stable` into a lazy initialiser that another module's getter
// never runs (f4caeb9d, scripts/wasm-stability-check.py).
@Stable
class PaneClock(private val read: () -> Double) {
    var now by mutableStateOf(read())
        private set
    var minute by mutableStateOf(now)
        private set
    private var ticks = 0

    fun tick() {
        now = read()
        if (++ticks % 60 == 0) minute = now
    }
}

@Composable
fun rememberPaneClock(read: () -> Double): PaneClock {
    val clock = remember { PaneClock(read) }
    LaunchedEffect(clock) {
        while (true) {
            delay(1000)
            clock.tick()
        }
    }
    return clock
}
