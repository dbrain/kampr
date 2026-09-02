package dev.kampr.terminal

import androidx.compose.ui.input.pointer.PointerIcon
import androidx.compose.ui.semantics.SemanticsNode

// Compose exposes no cursor to a test: the icon a node asks for is only ever an internal modifier
// element on that node's chain, and which one wins is a rule the platform applies at hover time.
// So this reads the chain and applies that rule itself — deepest node wins, unless an ancestor
// took the decision off it with `overrideDescendants`, in which case the highest one that did.
//
// A deliberate copy of `dev.kampr.shared.HoverChain`, and the only kind of duplication this repo
// has for it: this module may not depend on another module's *test* sources, and a helper that
// exists to read a test's own harness does not belong in a source set that ships to a phone.
private val HOVER = Regex("""PointerHoverIconModifierElement\(icon=(.+), overrideDescendants=(\w+)\)""")

private fun SemanticsNode.hoverChain(): List<Pair<String, Boolean>> =
    layoutInfo.getModifierInfo()
        .mapNotNull { HOVER.find(it.modifier.toString()) }
        .map { it.groupValues[1] to it.groupValues[2].toBoolean() } +
        children.flatMap { it.hoverChain() }

internal fun SemanticsNode.cursor(): String? {
    val chain = hoverChain()
    return chain.firstOrNull { it.second }?.first ?: chain.lastOrNull()?.first
}

internal val HAND = PointerIcon.Hand.toString()
internal val TEXT = PointerIcon.Text.toString()
internal val ARROW = PointerIcon.Default.toString()
