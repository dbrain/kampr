package dev.kampr.shared.ui

import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.isTraversalGroup
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.traversalIndex
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr

// The touch rule, from docs/04-wire-protocol.md: 44 dp with one hand, 36 dp for the two-thumb
// landscape posture where 44 would cost a quarter of a 390 dp-tall screen.
val TOUCH: Dp = 44.dp
val LANDSCAPE_TOUCH: Dp = 36.dp

private val FOCUS_RING = 2.dp

// A keyboard user gets nothing from a control that is reachable but invisible when reached, and
// nothing here draws a focus state of its own.
@Composable
private fun Modifier.focusRing(shape: Shape = RectangleShape, tint: Color = Kampr.tokens.color.accentHi): Modifier {
    var focused by remember { mutableStateOf(false) }
    return this
        .onFocusChanged { focused = it.isFocused }
        .then(if (focused) Modifier.border(FOCUS_RING, tint, shape) else Modifier)
}

// Every interactive control in Kampr goes through here. `label` names the action rather than the
// glyph — "Zoom, currently 1.6×", never "magnifier" — and it replaces whatever text the control
// paints, because a screen reader wants the action and the eye wants the abbreviation.
@Composable
fun Modifier.action(
    label: String,
    onClick: () -> Unit,
    shape: Shape = RectangleShape,
    role: Role = Role.Button,
    enabled: Boolean = true,
    selected: Boolean? = null,
    state: String? = null,
    onLongClick: (() -> Unit)? = null,
): Modifier = this
    .focusRing(shape)
    .semantics(mergeDescendants = true) {
        contentDescription = label
        this.role = role
        if (selected != null) this.selected = selected
        if (state != null) stateDescription = state
    }
    .clickable(enabled = enabled, onClick = onClick, onClickLabel = null)
    .then(
        if (onLongClick == null) Modifier
        else Modifier.semantics { onLongClick(label = null) { onLongClick(); true } },
    )

// A control driven by a raw gesture detector — the key row's caps, the terminal grid — reaches a
// screen reader only through explicit actions: TalkBack's double tap dispatches the semantic
// click, and a pointerInput block never sees it.
fun Modifier.gestureAction(
    label: String,
    onClick: () -> Unit,
    onLongClick: (() -> Unit)? = null,
    role: Role = Role.Button,
    state: String? = null,
    clickLabel: String? = null,
    longLabel: String? = null,
    actions: List<Pair<String, () -> Unit>> = emptyList(),
): Modifier = semantics(mergeDescendants = true) {
    contentDescription = label
    this.role = role
    if (state != null) stateDescription = state
    onClick(label = clickLabel) { onClick(); true }
    if (onLongClick != null) onLongClick(label = longLabel) { onLongClick(); true }
    if (actions.isNotEmpty()) {
        customActions = actions.map { (name, run) -> CustomAccessibilityAction(name) { run(); true } }
    }
}

// A name for something that is not a control: a status mark, a strip of chrome, a region.
fun Modifier.named(label: String): Modifier = semantics { contentDescription = label }

// Something that appears without the reader asking for it — a prompt, an error, a node dropping
// off the mesh. Polite waits for a gap in speech; urgent interrupts, and is reserved for the two
// things that hold the operator up: a blocked agent and a command about to run.
fun Modifier.announce(label: String, urgent: Boolean = false): Modifier =
    semantics(mergeDescendants = true) {
        contentDescription = label
        liveRegion = if (urgent) LiveRegionMode.Assertive else LiveRegionMode.Polite
    }

// Traversal order is composition order, and three of Kampr's screens paint the terminal first and
// the chrome over it — so reading order and composition order disagree unless this says otherwise.
fun Modifier.readingOrder(index: Float): Modifier = semantics {
    isTraversalGroup = true
    traversalIndex = index
}

fun Modifier.group(): Modifier = semantics { isTraversalGroup = true }

fun Modifier.asHeading(): Modifier = semantics { heading() }

// The painted glyph stays small so a header does not grow around it; the box that catches the tap
// is the one the touch rule sizes. Everything icon-only in Kampr is one of these.
@Composable
fun GlyphTarget(
    icon: Icon,
    label: String,
    tint: Color,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    target: Dp = TOUCH,
    glyph: Dp = 17.dp,
    enabled: Boolean = true,
) {
    Box(
        modifier.size(target).action(label, onClick, enabled = enabled),
        contentAlignment = Alignment.Center,
    ) {
        IconGlyph(icon, glyph, tint)
    }
}

@Composable
fun BackAction(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    target: Dp = TOUCH,
    tint: Color = Kampr.tokens.color.dim,
) = GlyphTarget(KamprIcons.chevronLeft, label, tint, onClick, modifier, target)

@Composable
fun CloseAction(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    target: Dp = TOUCH,
    tint: Color = Kampr.tokens.color.dim,
) = GlyphTarget(KamprIcons.cross, label, tint, onClick, modifier, target, glyph = 18.dp)

// Keeps a control that sizes itself from its content above the touch floor without changing what
// it paints.
fun Modifier.touchable(target: Dp = TOUCH): Modifier = defaultMinSize(minHeight = target)
