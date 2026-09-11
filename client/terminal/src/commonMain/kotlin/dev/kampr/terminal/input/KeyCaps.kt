package dev.kampr.terminal.input

enum class CapKind { Text, Latch, Keyboard, Blank }

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

// A slot with nothing in it, so the navigation group stays in the same columns on every row.
private val blank = KeyCap("", CapKind.Blank)

private val escape = text("esc", Esc.ESCAPE)

// Every modifier has a cap. Shift and fn both used to ride a long press — shift on ctrl, fn on
// alt — because every slot was spoken for, and a key whose only way in is an unlabelled long press
// is a key nobody finds.
private val ctrl = latch("ctrl", Latch.Ctrl)
private val alt = latch("alt", Latch.Alt)
private val shift = latch("shift", Latch.Shift)
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

private fun fn(n: Int) = csi("F$n", Esc.function(n))

// null is the fixed separator track between the modifier group and the navigation group.
typealias KeyRowSpec = List<KeyCap?>

// A cap paints two or three characters because that is all a 44 dp square holds. None of them is
// what the key is called, and an arrow read aloud as "up arrow" is the difference between a key row
// a screen reader can drive and a row of forty unnamed buttons.
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
)

private val FUNCTION = Regex("^F(\\d{1,2})$")

fun spokenKey(label: String): String =
    SPOKEN[label] ?: FUNCTION.matchEntire(label)?.let { "F " + it.groupValues[1] } ?: label

object KeyLayouts {
    // `fn` takes the slot `-` had, on the operator's own reading of the row: a hyphen and an
    // underscore are both on the soft keyboard's first symbol page and the function keys are on
    // nothing at all. It sits in the **same slot on the layer it turns on**, beside `kbd`, so
    // pressing it twice is two presses in one place.
    //
    // **Nothing here types a character the soft keyboard already has.** A cap writes to the pane
    // past the buffer the keyboard reads its suggestions and corrections from, so the field lets
    // go of the line it was mirroring (`FieldTextInput`) — a `/` pressed here cost the operator the
    // word in front of it, where the keyboard's own `/` kept it. `/` and `|` gave their slots to
    // shift and to nothing, and landscape's row of eight symbols went with them.
    val portrait: List<KeyRowSpec> = listOf(
        listOf(escape, ctrl, alt, tab, null) + navTop,
        listOf(shift, blank, fnKey, keyboard, null) + navBottom,
    )

    // **Twelve caps, and the modifiers they are pressed with.** The operator: *"we have a `fn`
    // button but it only gives me F1-F6"* and *"probably also needs to not replace existing
    // buttons so I could alt+f4 for example"*. F7 to F12 were here, as the alternate of the cap
    // six along, behind a long press nothing on the row named — which is the defect this layer was
    // built to fix one release earlier, one level down. A key nobody can see is a key nobody has.
    //
    // **A third row, and it is the only thing that fits.** Eight slots hold four function keys,
    // `fn`, `kbd` and the inverted T the arrows make on every layout here — leaving nothing for
    // the other eight function keys or for a modifier to press one with. The row this layer adds
    // is paid for only while the layer is up, and one tap puts it away; a long press is not paid
    // for at all, and it is what nobody found.
    //
    // F1 to F6 stay in the slots they were in, so a thumb that learned them keeps them, and `fn`
    // and `kbd` keep theirs.
    val portraitFn: List<KeyRowSpec> = listOf(
        listOf(fn(1), fn(2), fn(3), fn(4), null) + navTop,
        listOf(fn(5), fn(6), fnKey, keyboard, null) + navBottom,
        listOf(fn(7), fn(8), fn(9), fn(10), null, fn(11), fn(12), ctrl, alt),
    )

    val landscape: List<KeyRowSpec> = listOf(
        listOf(escape, ctrl, alt, tab, shift, blank, fnKey, keyboard, null) + navTop,
        List(8) { blank } + listOf(null) + navBottom,
    )

    // Twelve across a row that has the width for them, and the two modifiers a chord takes. `ins`
    // and `del` are not lost: they are what `home` and `end` hold, on every layout including this
    // one — and `esc` and the back-tab that used to sit in these two
    // slots are on the layer below, which is where the keys this layer is not about belong.
    val landscapeFn: List<KeyRowSpec> = listOf(
        listOf(fn(1), fn(2), fn(3), fn(4), fn(5), fn(6), fnKey, keyboard, null) + navTop,
        listOf(
            fn(7), fn(8), fn(9), fn(10), fn(11), fn(12), ctrl, alt,
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
