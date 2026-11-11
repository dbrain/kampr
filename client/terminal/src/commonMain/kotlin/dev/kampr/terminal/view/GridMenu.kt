package dev.kampr.terminal.view

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.Icon
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LocalSheetWide
import dev.kampr.shared.ui.MenuRow
import dev.kampr.shared.ui.edge

private val MENU_WIDTH = 200.dp

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
    CompositionLocalProvider(LocalSheetWide provides true) {
        Column(
            Modifier
                .atPixels(at.x, at.y)
                .width(MENU_WIDTH)
                .background(tokens.color.raise, RoundedCornerShape(tokens.radii.md))
                .edge(tokens.card, RoundedCornerShape(tokens.radii.md))
                .padding(vertical = 4.dp),
        ) {
            if (onCopy != null) Item("Copy", KamprIcons.copyText, "Copy the selection to the clipboard", onCopy, onDismiss)
            if (onPaste != null) Item("Paste", KamprIcons.pasteText, "Paste the clipboard into this pane", onPaste, onDismiss)
            Item("Select all", KamprIcons.selectAll, "Select the whole grid", onSelectAll, onDismiss)
        }
    }
}

@Composable
private fun Item(text: String, icon: Icon, spoken: String, onAct: () -> Unit, onDismiss: () -> Unit) {
    MenuRow(
        text = text,
        onClick = {
            onAct()
            onDismiss()
        },
        icon = icon,
        label = spoken,
    )
}
