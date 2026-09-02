package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg

// Herdr's sidebar menu is two items long and both of them are about the row itself (#426). Kampr's
// rows are panes rather than workspaces, so the verbs are a pane's — and the list stays that short
// for the same reason herdr's does: a pane in a list is usually a pane nobody is looking at, and
// every one of the in-session sheet's desk controls is either meaningless or a lie when you cannot
// see the screen it reshapes. Nothing here focuses, selects or claims the row it was opened on;
// opening the menu puts nothing on the wire at all, which is rule 3 and is what #426 measured
// herdr doing — the tab strip never moved off the focused workspace while an unfocused one's menu
// was open.
private val MENU_WIDTH = 216.dp
private val MENU_EDGE = 8.dp

private enum class MenuStep { Verbs, Rename, Close }

@Composable
fun PaneMenu(
    breakpoint: Breakpoint,
    pane: PaneInfo,
    anchor: MenuAnchor?,
    outcome: ServerMsg.Managed?,
    onManage: (ManageOp) -> Unit,
    onOpen: () -> Unit,
    onDismiss: () -> Unit,
) {
    val tokens = Kampr.tokens
    val title = paneTitle(pane)
    var refusal by remember { mutableStateOf<String?>(null) }
    var step by remember(pane.id) { mutableStateOf(MenuStep.Verbs) }
    var label by remember(pane.id) { mutableStateOf(TextFieldValue(pane.label ?: pane.id.substringAfter('/'))) }

    LaunchedEffect(outcome) {
        val ack = outcome ?: return@LaunchedEffect
        if (ack.ok) onDismiss() else refusal = ack.message ?: ack.code
    }

    val body: @Composable ColumnScope.() -> Unit = {
        when (step) {
            MenuStep.Verbs -> {
                MenuItem("Open", "Open $title", onClick = onOpen)
                MenuItem("Rename…", "Rename $title") { step = MenuStep.Rename }
                MenuItem("Close", "Close $title", danger = true) { step = MenuStep.Close }
            }
            MenuStep.Rename -> {
                MenuNote("Rename pane")
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 12.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    KField("name", label, Modifier.weight(1f), label = "New name for $title") { label = it }
                }
                MenuNote("An empty name puts back what the pane calls itself.")
                MenuRow {
                    Chip("cancel", false, { step = MenuStep.Verbs }, quiet = true, label = "Leave the name alone")
                    Chip(
                        "save",
                        false,
                        { onManage(ManageOp.Rename(pane.id, label.text.trim().ifEmpty { null })) },
                        label = "Save the new name for $title",
                    )
                }
            }
            MenuStep.Close -> {
                // Herdr names the thing and counts it before it will act — `Close workspace?` over
                // `bravo — 1 pane` (#426). One pane is one pane, so the count is the name.
                Column(Modifier.announce("Close pane? $title. Confirm or cancel.", urgent = true)) {
                    MenuNote("Close pane?")
                    MenuNote(title, loud = true)
                }
                MenuRow {
                    Chip("cancel", false, { step = MenuStep.Verbs }, quiet = true, label = "Do not close $title")
                    Chip("close", true, { onManage(ManageOp.Close(pane.id)) }, label = "Confirm — close $title")
                }
            }
        }
        refusal?.let {
            KText(
                it,
                tokens.type.captionSmall,
                tokens.color.blocked,
                Modifier.padding(horizontal = 14.dp, vertical = 4.dp).announce(it, urgent = true),
                maxLines = 3,
            )
        }
    }

    // A finger has no pointer to hang a box off, and a 216 dp card floating mid-screen is not what
    // a phone means by a menu. The same fallback catches a screen reader's long press on a desk,
    // which arrives as a semantic action with no position at all.
    if (breakpoint == Breakpoint.Portrait || anchor == null) {
        BottomSheet(breakpoint, onDismiss) {
            SheetHeader("Menu", title, null, onDismiss, compact = true)
            Column(Modifier.padding(bottom = 14.dp), content = body)
        }
    } else {
        AnchoredMenu(anchor, onDismiss, body)
    }
}

@Composable
private fun AnchoredMenu(
    anchor: MenuAnchor,
    onDismiss: () -> Unit,
    content: @Composable ColumnScope.() -> Unit,
) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Box(Modifier.fillMaxSize()) {
        // A left click anywhere else closes it and opens nothing (#426) — caught, but not dimmed:
        // the sheet's scrim is right behind a 420 dp panel and wrong behind a 216 dp box that is
        // meant to read as hanging over the row it came from.
        Box(
            Modifier
                .fillMaxSize()
                .gestureAction("Close the menu", onClick = onDismiss)
                .clickable(remember { MutableInteractionSource() }, indication = null, onClick = onDismiss)
        )
        BoxWithConstraints {
            var size by remember { mutableStateOf(IntSize.Zero) }
            val tall = with(LocalDensity.current) { size.height.toDp() }
            val room = maxWidth
            val x = anchor.x.coerceIn(MENU_EDGE, (room - MENU_WIDTH - MENU_EDGE).coerceAtLeast(MENU_EDGE))
            // The box flips upward rather than off the bottom (#426). Zero on the first frame,
            // which places it at the pointer and corrects once it has been measured.
            val y =
                if (anchor.y + tall + MENU_EDGE > maxHeight) (anchor.y - tall).coerceAtLeast(MENU_EDGE)
                else anchor.y
            Column(
                Modifier
                    .offset(x, y)
                    .width(MENU_WIDTH)
                    .onSizeChanged { size = it }
                    .background(tokens.color.bar, shape)
                    .modal(onDismiss)
                    .edge(tokens.chrome, shape)
                    .padding(vertical = 5.dp),
                content = content,
            )
        }
    }
}

@Composable
private fun MenuItem(text: String, label: String, danger: Boolean = false, onClick: () -> Unit) {
    val tokens = Kampr.tokens
    Box(
        Modifier
            .fillMaxWidth()
            .touchable(LANDSCAPE_TOUCH)
            .action(label, onClick)
            .padding(horizontal = 14.dp, vertical = 9.dp),
        contentAlignment = Alignment.CenterStart,
    ) {
        // A verb, not prose. The menu falls back to a `BottomSheet` on a phone and wherever there
        // is no pointer to hang a box off, and that sheet selects — so without this a drag across
        // it copies "Open" and "Close" out from between the words worth quoting. `MenuNote` below
        // is the other half of the same call: it carries the pane's own title, which is content.
        DisableSelection {
            KText(text, tokens.type.body, if (danger) tokens.color.blocked else tokens.color.text, maxLines = 1)
        }
    }
}

@Composable
private fun MenuNote(text: String, loud: Boolean = false) {
    val tokens = Kampr.tokens
    KText(
        text,
        if (loud) tokens.type.body else tokens.type.captionSmall,
        if (loud) tokens.color.text else tokens.color.mute,
        Modifier.padding(horizontal = 14.dp, vertical = 4.dp),
        maxLines = 2,
    )
}

@Composable
private fun MenuRow(content: @Composable () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        content()
    }
}
