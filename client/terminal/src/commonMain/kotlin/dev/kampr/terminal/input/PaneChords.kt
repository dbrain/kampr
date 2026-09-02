package dev.kampr.terminal.input

// The two chords a terminal surface may not hand to the pane, and the one table that says so.
//
// **ctrl+shift+C is the copy chord on every Linux terminal emulator**, and it arrived here as
// `e.key === "C"`, was lowercased to `c` and went to the pane as `^C`: copying interrupted the
// process. ctrl+shift+V became `^V` the same way. On macOS `⌘C` hit the same branch, which made
// Command-C a SIGINT — a ⌘ chord is not a control code on any platform, and `⌘T`, `⌘W` and `⌘L`
// belong to the browser rather than to Kampr, so nothing here may swallow one either.
//
// What does not change: ctrl+C without shift is still `^C`, which is the whole point of a terminal,
// and every other `ctrl+shift+<letter>` still produces its own control byte, which is what a
// terminal does with them. Only C and V are taken.
enum class PaneChord { Copy, Paste }

fun paneChord(key: Char, ctrl: Boolean, meta: Boolean, shift: Boolean): PaneChord? {
    val wanted = when (key.lowercaseChar()) {
        'c' -> PaneChord.Copy
        'v' -> PaneChord.Paste
        else -> return null
    }
    if (meta) return wanted
    return if (ctrl && shift) wanted else null
}

// Whether a modified single character may still become a control byte at all. Ctrl is the only
// modifier that makes one; a chord carrying the platform's command key is the platform's, and
// turning `⌘T` into `^T` both interrupted the pane and stole the browser's new tab.
fun chordSendsControl(ctrl: Boolean, meta: Boolean): Boolean = ctrl && !meta
