package dev.kampr.shared

import androidx.compose.ui.input.pointer.PointerIcon
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull

// Compose exposes no cursor to a test: the icon a node asks for is only ever an internal modifier
// element on that node's chain, and which one wins is a rule the platform applies at hover time.
// So this reads the chain and applies that rule itself — deepest node wins, unless an ancestor
// took the decision off it with `overrideDescendants`, in which case the highest one that did.
internal val HOVER = Regex("""PointerHoverIconModifierElement\(icon=(.+), overrideDescendants=(\w+)\)""")

internal fun SemanticsNode.hoverChain(): List<Pair<String, Boolean>> =
    layoutInfo.getModifierInfo()
        .mapNotNull { HOVER.find(it.modifier.toString()) }
        .map { it.groupValues[1] to it.groupValues[2].toBoolean() } +
        children.flatMap { it.hoverChain() }

internal fun SemanticsNode.cursor(): String? {
    val chain = hoverChain()
    return chain.firstOrNull { it.second }?.first ?: chain.lastOrNull()?.first
}

internal fun SemanticsNode.paintsText(): Boolean =
    children.any { it.config.getOrNull(SemanticsProperties.Text) != null || it.paintsText() }

internal val HAND = PointerIcon.Hand.toString()
internal val TEXT = PointerIcon.Text.toString()
internal val ARROW = PointerIcon.Default.toString()

// **Whether a run of painted text is one a drag across the screen would carry off.** There is no
// direct read for it — `DisableSelection` works by providing a null `LocalSelectionRegistrar`, and
// a composition local leaves no trace on a node — but `BasicText` decides both things in the same
// branch: `selectionRegistrar != null` is what builds the `selectionController` that registers the
// run with the container *and* what pins the I-beam onto the text's own layout node. So the icon on
// the text node is the registration, seen from the outside. That is also the shape of the report
// this rule came from: a chip that hovered as prose was a chip whose caption a drag could take.
//
// Read off the text node itself and never off an ancestor: a control wears `Modifier.pressable`,
// whose `overrideDescendants` decides the *cursor* over the whole button and says nothing at all
// about whether the words inside it are selectable.
internal fun SemanticsNode.selectsItsOwnText(): Boolean =
    layoutInfo.getModifierInfo().any { info ->
        HOVER.find(info.modifier.toString())?.groupValues?.get(1) == TEXT
    }

// Every run of painted text at or under this node, with the string it paints.
internal fun SemanticsNode.textRuns(): List<Pair<String, SemanticsNode>> =
    (config.getOrNull(SemanticsProperties.Text)?.map { it.text to this } ?: emptyList()) +
        children.flatMap { it.textRuns() }
