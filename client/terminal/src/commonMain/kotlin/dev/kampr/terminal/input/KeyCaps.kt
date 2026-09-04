package dev.kampr.terminal.input

enum class CapKind { Text, Latch, Keyboard }

data class KeyCap(
    val label: String,
    val kind: CapKind = CapKind.Text,
    val send: String = "",
    val latch: Latch? = null,
    val alternate: KeyCap? = null,
    val hold: Latch? = null,
    val csi: Boolean = false,
    val symbol: Boolean = false,
    // Whether holding this cap repeats it, the way a physical keyboard's autorepeat does. Only the
    // arrows: every other cap on the row spends its long press on something already — an
    // alternate, a latch, a lock — and a repeat cannot share a gesture with any of them.
    val repeats: Boolean = false,
)

private fun text(label: String, send: String = label, alternate: KeyCap? = null) =
    KeyCap(label, CapKind.Text, send, alternate = alternate)

private fun csi(
    label: String,
    send: String,
    alternate: KeyCap? = null,
    symbol: Boolean = false,
    repeats: Boolean = false,
) = KeyCap(
    label, CapKind.Text, send,
    alternate = alternate, csi = true, symbol = symbol, repeats = repeats,
)

private fun latch(label: String, which: Latch, hold: Latch? = null) =
    KeyCap(label, CapKind.Latch, latch = which, hold = hold)

private val insert = csi("ins", Esc.INSERT)
private val delete = csi("del", Esc.DELETE)
private val keyboard = KeyCap("kbd", CapKind.Keyboard)

private val escape = text("esc", Esc.ESCAPE, alternate = text("~"))

// Shift latches on a long press of Ctrl: the row is eight columns wide, every one of them is
// spoken for by the artboard, and shift is the modifier this row's own keys already carry — the
// arrows and tab take it, and a letter needs the soft keyboard anyway.
//
// Fn used to ride on Alt the same way, and it is the one that could not. It does not modify the
// next key, it *replaces the row*, and a layer whose only way in is an unlabelled long press is a
// layer nobody finds — the operator asked for a way to see the function keys that were already
// there. So it has a cap.
private val ctrl = latch("ctrl", Latch.Ctrl, Latch.Shift)
private val alt = latch("alt", Latch.Alt)
private val fnKey = latch("fn", Latch.Fn)
private val tab = text("tab", Esc.TAB, alternate = csi("tab", Esc.BACKTAB))

private val home = csi("home", Esc.HOME, alternate = insert)
private val end = csi("end", Esc.END, alternate = delete)
private val pageUp = csi("pgup", Esc.PAGE_UP)
private val pageDown = csi("pgdn", Esc.PAGE_DOWN)

// The inverted T: up sits directly above down, with left and right flanking it, the way it is on
// every physical keyboard. An L-shape is what makes a thumb look down.
private val up = csi("↑", Esc.UP, symbol = true, repeats = true)
private val down = csi("↓", Esc.DOWN, symbol = true, repeats = true)
private val left = csi("←", Esc.LEFT, symbol = true, repeats = true)
private val right = csi("→", Esc.RIGHT, symbol = true, repeats = true)

private val navTop = listOf(home, pageUp, up, pageDown)
private val navBottom = listOf(end, left, down, right)

private fun fn(n: Int, alternate: Int? = null) =
    csi("F$n", Esc.function(n), alternate = alternate?.let { csi("F$it", Esc.function(it)) })

// F1-F12 across six slots, the upper six on a long press of the lower six. Regular rather than
// clever: the sixth key along is F6 and holding it is F12, so the pairing is one rule and not a
// table to memorise.
private fun fnPair(n: Int) = fn(n, n + 6)

// null is the fixed separator track between the modifier/symbol group and the navigation group.
typealias KeyRowSpec = List<KeyCap?>

// A cap paints two or three characters because that is all a 44 dp square holds. None of them is
// what the key is called, and a slash read aloud as "slash" is the difference between a key row a
// screen reader can drive and a row of forty unnamed buttons.
private val SPOKEN = mapOf(
    "esc" to "Escape",
    "ctrl" to "Control",
    "alt" to "Alt",
    "tab" to "Tab",
    "shift" to "Shift",
    "fn" to "Function",
    "kbd" to "Keyboard",
    "ins" to "Insert",
    "del" to "Delete",
    "home" to "Home",
    "end" to "End",
    "pgup" to "Page up",
    "pgdn" to "Page down",
    "\u2191" to "Up arrow",
    "\u2193" to "Down arrow",
    "\u2190" to "Left arrow",
    "\u2192" to "Right arrow",
    "/" to "Slash",
    "\\" to "Backslash",
    "|" to "Pipe",
    "~" to "Tilde",
    "&" to "Ampersand",
    "*" to "Asterisk",
    "$" to "Dollar",
    "\"" to "Double quote",
    "'" to "Apostrophe",
    "`" to "Backtick",
)

private val FUNCTION = Regex("^F(\\d{1,2})$")

fun spokenKey(label: String): String =
    SPOKEN[label] ?: FUNCTION.matchEntire(label)?.let { "F " + it.groupValues[1] } ?: label

object KeyLayouts {
    // `fn` takes the slot `-` had, on the operator's own reading of the row: a hyphen and an
    // underscore are both on the soft keyboard's first symbol page and the function keys are on
    // nothing at all. It sits in the **same slot on the layer it turns on**, beside `kbd`, so
    // pressing it twice is two presses in one place.
    val portrait: List<KeyRowSpec> = listOf(
        listOf(escape, ctrl, alt, tab, null) + navTop,
        listOf(text("/", "/", text("\\")), text("|", "|", text("&")), fnKey, keyboard, null) + navBottom,
    )

    val portraitFn: List<KeyRowSpec> = listOf(
        listOf(fnPair(1), fnPair(2), fnPair(3), fnPair(4), null) + navTop,
        listOf(fnPair(5), fnPair(6), fnKey, keyboard, null) + navBottom,
    )

    val landscape: List<KeyRowSpec> = listOf(
        listOf(
            escape, ctrl, alt, tab,
            text("/", "/", text("\\")), text("|", "|", text("&")), fnKey, keyboard,
            null,
        ) + navTop,
        listOf(
            text("~"), text("&"), text("*"), text("$"),
            text("\\"), text("\""), text("'"), text("`"),
            null,
        ) + navBottom,
    )

    // Twelve across a row that has the width for them, so nothing here is a long press. `ins` and
    // `del` go with the symbols and are not lost: they are what `home` and `end` hold, on every
    // layout including this one.
    val landscapeFn: List<KeyRowSpec> = listOf(
        listOf(fn(1), fn(2), fn(3), fn(4), fn(5), fn(6), fnKey, keyboard, null) + navTop,
        listOf(
            fn(7), fn(8), fn(9), fn(10), fn(11), fn(12), csi("tab", Esc.BACKTAB), escape,
            null,
        ) + navBottom,
    )

    fun rows(compact: Boolean, fn: Boolean): List<KeyRowSpec> = when {
        compact && fn -> landscapeFn
        compact -> landscape
        fn -> portraitFn
        else -> portrait
    }
}
