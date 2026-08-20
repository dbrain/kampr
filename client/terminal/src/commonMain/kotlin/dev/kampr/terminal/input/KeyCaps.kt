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
)

private fun text(label: String, send: String = label, alternate: KeyCap? = null) =
    KeyCap(label, CapKind.Text, send, alternate = alternate)

private fun csi(label: String, send: String, alternate: KeyCap? = null, symbol: Boolean = false) =
    KeyCap(label, CapKind.Text, send, alternate = alternate, csi = true, symbol = symbol)

private fun latch(label: String, which: Latch, hold: Latch) =
    KeyCap(label, CapKind.Latch, latch = which, hold = hold)

private val insert = csi("ins", Esc.INSERT)
private val delete = csi("del", Esc.DELETE)
private val keyboard = KeyCap("kbd", CapKind.Keyboard)

private val escape = text("esc", Esc.ESCAPE, alternate = text("~"))

// Shift and Fn latch on a long press of Ctrl and Alt. The row is eight columns wide and every one
// of them is spoken for by the artboard, so the two rarer modifiers ride on the two common ones.
private val ctrl = latch("ctrl", Latch.Ctrl, Latch.Shift)
private val alt = latch("alt", Latch.Alt, Latch.Fn)
private val tab = text("tab", Esc.TAB, alternate = csi("tab", Esc.BACKTAB))

private val home = csi("home", Esc.HOME, alternate = insert)
private val end = csi("end", Esc.END, alternate = delete)
private val pageUp = csi("pgup", Esc.PAGE_UP)
private val pageDown = csi("pgdn", Esc.PAGE_DOWN)

// The inverted T: up sits directly above down, with left and right flanking it, the way it is on
// every physical keyboard. An L-shape is what makes a thumb look down.
private val up = csi("↑", Esc.UP, symbol = true)
private val down = csi("↓", Esc.DOWN, symbol = true)
private val left = csi("←", Esc.LEFT, symbol = true)
private val right = csi("→", Esc.RIGHT, symbol = true)

private val navTop = listOf(home, pageUp, up, pageDown)
private val navBottom = listOf(end, left, down, right)

private fun fn(n: Int, alternate: Int? = null) =
    csi("F$n", Esc.function(n), alternate = alternate?.let { csi("F$it", Esc.function(it)) })

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
    "-" to "Hyphen",
    "_" to "Underscore",
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
    val portrait: List<KeyRowSpec> = listOf(
        listOf(escape, ctrl, alt, tab, null) + navTop,
        listOf(text("/", "/", text("\\")), text("|", "|", text("&")), text("-", "-", text("_")), keyboard, null) + navBottom,
    )

    val portraitFn: List<KeyRowSpec> = listOf(
        listOf(fn(1, 9), fn(2, 10), fn(3, 11), fn(4, 12), null) + navTop,
        listOf(fn(5), fn(6), fn(7), fn(8), null) + navBottom,
    )

    val landscape: List<KeyRowSpec> = listOf(
        listOf(
            escape, ctrl, alt, tab,
            text("/", "/", text("\\")), text("|", "|", text("&")), text("-", "-", text("_")), keyboard,
            null,
        ) + navTop,
        listOf(
            text("~"), text("&"), text("*"), text("$"),
            text("\\"), text("\""), text("'"), text("`"),
            null,
        ) + navBottom,
    )

    val landscapeFn: List<KeyRowSpec> = listOf(
        listOf(fn(1), fn(2), fn(3), fn(4), fn(5), fn(6), fn(7), fn(8), null) + navTop,
        listOf(
            fn(9), fn(10), fn(11), fn(12), insert, delete, csi("tab", Esc.BACKTAB), escape,
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
