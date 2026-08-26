package dev.kampr.shared.platform

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf

// Three answers collapsed onto two, and deliberately onto the side that costs least. "No keyboard"
// and "this platform cannot tell" are the same value here, because the two mistakes are not the
// same size: a key row shown to someone with a keyboard is a strip of clutter along the bottom of
// a window that has room for it, and a key row hidden from a tablet that has none leaves an
// operator with no Escape, no Ctrl and no arrows — no way out of vim and no way to answer a prompt.
//
// So every platform's uncertainty falls to `false`, and only a positive reading returns `true`.
@Composable
expect fun hardKeyboardAttached(): Boolean

// Provided once at the app root from `hardKeyboardAttached()`, so that a layout reads a value and
// a test can provide one — which is the only way this suite can see the case the report is about,
// since the JVM actual is a constant.
val LocalHardKeyboard: ProvidableCompositionLocal<Boolean> = staticCompositionLocalOf { false }

// Monotone, and that is the whole point of it. Every platform's reading can move: Android's
// configuration moves when a keyboard case is docked, and a browser's is a guess about a machine
// it cannot see. The two directions are not symmetrical. Growing a key row that was not needed
// puts a spare strip of caps along the bottom of the window; taking one away leaves whoever was
// looking at it with no Escape, no arrows and no latches, mid-task, and the report that produced
// this rule described exactly that — caps that "stay for a while then go away weirdly".
//
// So evidence may only ever add the row. Once anything says there is no keyboard, this pane keeps
// its row for as long as it is on screen, and a keyboard noticed afterwards does not undo it. This
// reading can be wrong, and this is the direction it is allowed to be wrong in.
@Composable
fun keyRowNeeded(): Boolean {
    var needed by remember { mutableStateOf(false) }
    if (!LocalHardKeyboard.current) needed = true
    return needed
}
