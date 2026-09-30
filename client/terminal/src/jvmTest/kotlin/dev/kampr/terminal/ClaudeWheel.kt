package dev.kampr.terminal

import kotlin.math.min

// Claude 2.1.285's wheel arithmetic in "window (native)" mode, as read out of its binary and
// measured to the row (#567, #569), fed the time each write *arrives*. It is the judge the
// pump is held to rather than the model the pump sizes its writes by: a pump that sends a write
// into the wrong side of the 40 ms window is caught here however its own arithmetic agrees
// with itself. Reports only ever go out in one direction per write, and a mouse that bounces back
// inside 200 ms of a reversal is a mode no finger reaches, so meeting one fails the test.
class ClaudeWheel {
    private var time = Long.MIN_VALUE / 2
    private var mult = 1.0
    private var dir = 0
    private var flip = false
    var rows = 0
        private set

    fun write(at: Long, text: String) {
        var i = text.indexOf("\u001b[<")
        while (i >= 0) {
            report(at, if (text.startsWith("\u001b[<64;", i)) 1 else -1)
            i = text.indexOf("\u001b[<", i + 1)
        }
    }

    private fun report(at: Long, direction: Int) {
        if (flip) {
            flip = false
            check(direction != dir || at - time > 200) { "a report bounced back inside Claude's flip window" }
            dir = direction
            time = at
            mult = 1.0
            rows += direction
            return
        }
        val gap = at - time
        if (direction != dir && dir != 0) {
            flip = true
            time = at
            return
        }
        dir = direction
        time = at
        mult = if (gap > 40) 1.0 else min(6.0, mult + 0.3)
        rows += direction * mult.toInt()
    }
}
