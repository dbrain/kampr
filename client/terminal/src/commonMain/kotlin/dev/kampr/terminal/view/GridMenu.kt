package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.touchable

// What a right-click over the grid opens, which was nothing at all. `Modifier.paneActions` is on
// the sidebar cards, the herd list and the mosaic cells; the one surface a desk actually
// right-clicks — the text — had no menu, and `boot.js` was already suppressing the browser's own
// over the canvas, so the gesture landed on a surface that had taken its alternative away.
//
// The same three verbs the selection pill carries, in the shape a mouse expects and at the pointer
// rather than at the selection: a right-click with nothing selected still wants Select all and
// Paste, and the pill only exists once there is something to copy.
//
// Paste is absent on a read-only device rather than present and refusing, which is the rule
// `SelectionLayer` and `ManageLayer` already follow for everything that writes. Copy is absent
// while there is nothing selected, for the same reason: an item that cannot do anything is worse
// than one that is not there.
@Composable
internal fun GridMenu(
    at: Offset,
    onCopy: (() -> Unit)?,
    onPaste: (() -> Unit)?,
    onSelectAll: () -> Unit,
    onDismiss: () -> Unit,
) {
    val tokens = Kampr.tokens
    Box(
        Modifier
            .fillMaxSize()
            .pointerInput(Unit) { detectTapGestures { onDismiss() } },
    )
    Column(
        Modifier
            .atPixels(at.x, at.y)
            .background(tokens.color.raise, RoundedCornerShape(tokens.radii.md))
            .edge(tokens.card, RoundedCornerShape(tokens.radii.md)),
    ) {
        if (onCopy != null) Item("Copy", "Copy the selection to the clipboard", onCopy, onDismiss)
        if (onPaste != null) Item("Paste", "Paste the clipboard into this pane", onPaste, onDismiss)
        Item("Select all", "Select the whole grid", onSelectAll, onDismiss)
    }
}

@Composable
private fun Item(label: String, spoken: String, onAct: () -> Unit, onDismiss: () -> Unit) {
    val tokens = Kampr.tokens
    Box(
        Modifier
            .touchable()
            .action(spoken, onClick = {
                onAct()
                onDismiss()
            })
            .padding(horizontal = 14.dp, vertical = 9.dp),
    ) {
        KText(label, tokens.type.buttonSmall, tokens.color.text)
    }
}
