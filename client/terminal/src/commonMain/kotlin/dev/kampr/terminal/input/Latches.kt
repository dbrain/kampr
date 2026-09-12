package dev.kampr.terminal.input

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

enum class Latch { Ctrl, Alt, Shift, Fn }

enum class LatchState { Off, Armed, Locked }

@Stable
class Latches {
    var ctrl by mutableStateOf(LatchState.Off)
        private set
    var alt by mutableStateOf(LatchState.Off)
        private set
    var shift by mutableStateOf(LatchState.Off)
        private set
    var fn by mutableStateOf(LatchState.Off)
        private set

    operator fun get(latch: Latch): LatchState = when (latch) {
        Latch.Ctrl -> ctrl
        Latch.Alt -> alt
        Latch.Shift -> shift
        Latch.Fn -> fn
    }

    private fun set(latch: Latch, value: LatchState) {
        when (latch) {
            Latch.Ctrl -> ctrl = value
            Latch.Alt -> alt = value
            Latch.Shift -> shift = value
            Latch.Fn -> fn = value
        }
    }

    // Tap arms the next keystroke, tap again locks it, tap again clears — long-press jumps
    // straight to locked, which is what a run of ctrl chords wants.
    //
    // **Except `fn`, which is a layer and not a prefix.** It has no armed state to be in:
    // [`consume`] leaves it standing where it clears the other three, and the row reads nothing
    // but `active()` — so `Armed` and `Locked` draw the same cap over the same layer, and the
    // third state cost a press moving between two nothing can tell apart. The operator: *"when I
    // press fn to get out of fn I need to press fn twice"*.
    fun tap(latch: Latch) = when (latch) {
        Latch.Fn -> lock(latch)
        else -> set(
            latch,
            when (this[latch]) {
                LatchState.Off -> LatchState.Armed
                LatchState.Armed -> LatchState.Locked
                LatchState.Locked -> LatchState.Off
            },
        )
    }

    fun lock(latch: Latch) = set(
        latch,
        if (this[latch] == LatchState.Locked) LatchState.Off else LatchState.Locked,
    )

    fun consume() {
        if (ctrl == LatchState.Armed) ctrl = LatchState.Off
        if (alt == LatchState.Armed) alt = LatchState.Off
        if (shift == LatchState.Armed) shift = LatchState.Off
    }

    fun clear() {
        ctrl = LatchState.Off
        alt = LatchState.Off
        shift = LatchState.Off
        fn = LatchState.Off
    }
}

fun LatchState.active(): Boolean = this != LatchState.Off
