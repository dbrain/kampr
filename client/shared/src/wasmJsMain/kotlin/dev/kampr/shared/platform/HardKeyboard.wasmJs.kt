package dev.kampr.shared.platform

import androidx.compose.runtime.Composable
import kotlin.js.ExperimentalWasmJsInterop

// Three answers, because the browser gives three and folding the third into either of the others
// is what shipped a key row onto a desk. `pointer` and `hover` describe the *primary* input
// device, which is the one the person is using:
//
//   - a desk browser is `(hover: hover) and (pointer: fine)` — a mouse or a trackpad, and a
//     machine with one of those has a keyboard under it. A laptop with a touchscreen and a
//     desktop whose driver reports a digitiser both land here, correctly, because the pointer
//     they are *driven* by is still fine.
//   - a phone or tablet browser is `(pointer: coarse)` — a fingertip, and no keys.
//   - and a browser can report neither. ChromeHeadless reports `hover: none`, `pointer: none`,
//     `any-pointer: none` and `maxTouchPoints: 0` (#266). That is a harness with no input device
//     attached to it rather than a machine anyone is sitting at, and it is its own answer, not a
//     quiet vote for one of the other two.
//
// `maxTouchPoints` was the previous reading and it is not the question. It is a hardware count,
// not a statement about how the machine is driven, and desktops and laptops report a non-zero one
// from a digitiser or a driver — so it read an operator's desk as a phone. It was chosen because
// it puts the *harness* on the desk side, and #266 is what that costs: the test drove the design
// and the design was wrong about every machine but the test's.
internal enum class PointerKind { Desk, Touch, Unknown }

@OptIn(ExperimentalWasmJsInterop::class)
private fun mediaMatches(query: String): Boolean = js(
    """
    (function () {
        try { return window.matchMedia(query).matches; } catch (e) { return false; }
    })()
    """
)

internal fun pointerKind(): PointerKind = when {
    mediaMatches("(hover: hover) and (pointer: fine)") -> PointerKind.Desk
    mediaMatches("(pointer: coarse)") -> PointerKind.Touch
    else -> PointerKind.Unknown
}

// Whether focusing an offscreen input would raise a keyboard over the page. Only a coarse pointer
// has one to raise, so `Unknown` is not touch: a browser that reports no input device has no
// keyboard to put over the pane, and refusing the focus there is a pane that takes no keys at all.
fun touchBrowser(): Boolean = pointerKind() == PointerKind.Touch

// The other question, and deliberately not the same threshold. There is no browser API for "is a
// physical keyboard attached" — `navigator.keyboard` is the Keyboard Map and Keyboard Lock API,
// it answers what a key prints and whether the page may take Escape, and Chrome for Android
// exposes it with no keyboard within a metre of the device — so this is a guess, and only a
// positive reading of a desk counts as one. `Unknown` keeps the key row, because a spare strip of
// caps costs a reader some clutter and a missing one costs an operator their only Escape.
//
// No evidence gathered after this. A keydown latch stood here and it could only ever move the
// reading towards "there is a keyboard", which is the direction that takes the row off the screen
// mid-session — the regression this replaces. `keyRowNeeded` holds the row whatever this says
// later, so evidence in that direction now has nothing left to do.
internal fun deskBrowser(): Boolean = pointerKind() == PointerKind.Desk

@Composable
actual fun hardKeyboardAttached(): Boolean = deskBrowser()
