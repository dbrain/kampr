package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.NativeAction
import dev.kampr.shared.model.isNative
import dev.kampr.shared.model.nativeActions
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.isMuffit

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
    desk: Boolean = false,
) {
    val title = paneTitle(pane)
    var refusal by remember { mutableStateOf<String?>(null) }
    var step by remember(pane.id) { mutableStateOf(MenuStep.Verbs) }
    val native = pane.isNative || pane.isMuffit
    val actions = if (native) nativeActions(pane, desk) else emptyList()
    val closing = if (native) actions.firstOrNull { it.confirm } else NativeAction("close", "Close $title", ManageOp.Close(pane.id), true)
    val kind = if (native) "session" else "pane"
    val compact = breakpoint != Breakpoint.Portrait

    LaunchedEffect(outcome) {
        val ack = outcome ?: return@LaunchedEffect
        if (ack.ok) onDismiss() else refusal = ack.message ?: ack.code
    }

    val verbs: @Composable ColumnScope.() -> Unit = {
        MenuRow("Open", onOpen, KamprIcons.open, label = "Open $title")
        if (!pane.isMuffit) MenuRow("Rename…", { step = MenuStep.Rename }, KamprIcons.pencil, label = "Rename $title")
        actions.filterNot { it.confirm }.forEach { action ->
            MenuRow(action.verb.replaceFirstChar(Char::uppercase), { onManage(action.op) }, KamprIcons.desk, label = action.spoken)
        }
        closing?.let { action ->
            MenuDivider()
            MenuRow(
                action.verb.replaceFirstChar(Char::uppercase) + "…",
                { step = MenuStep.Close },
                if (action.verb == "forget") KamprIcons.forget else KamprIcons.cross,
                danger = true,
                label = action.spoken,
            )
        }
        Refusal(refusal)
    }

    // A finger has no pointer to hang a box off, and a 216 dp card floating mid-screen is not what
    // a phone means by a menu. The same fallback catches a screen reader's long press on a desk,
    // which arrives as a semantic action with no position at all. A step past the verbs is a
    // question with buttons, which is a dialog wherever it is asked.
    if (step == MenuStep.Verbs && breakpoint != Breakpoint.Portrait && anchor != null) {
        AnchoredMenu(anchor, onDismiss, verbs)
        return
    }
    BottomSheet(breakpoint, onDismiss, wideWidth = DIALOG_WIDTH) {
        val back = { step = MenuStep.Verbs }
        when (step) {
            MenuStep.Verbs -> {
                SheetHeader(title, null, null, onDismiss, compact, dismiss = "Close the menu")
                Column(Modifier.padding(bottom = 12.dp), content = verbs)
            }

            MenuStep.Rename -> {
                RenamePage(kind, title, pane.label ?: pane.id.substringAfter('/'), clearable = true, compact, back, onDismiss) {
                    onManage(ManageOp.Rename(pane.id, it))
                }
                Refusal(refusal)
            }

            // Herdr names the thing and counts it before it will act — `Close workspace?` over
            // `bravo — 1 pane` (#426). One pane is one pane, so the count is the name.
            MenuStep.Close -> closing?.let { action ->
                val verb = action.verb.replaceFirstChar(Char::uppercase)
                ConfirmPage(
                    title = "$verb this $kind?",
                    subject = title,
                    detail = closeDetail(action.verb, kind),
                    confirm = "$verb $kind",
                    compact = compact,
                    onBack = back,
                    onDismiss = onDismiss,
                    onConfirm = { onManage(action.op) },
                )
                Refusal(refusal)
            }
        }
    }
}

internal fun closeDetail(verb: String, kind: String): String = when {
    verb == "forget" -> "Its saved output goes with it."
    kind == "pane" || kind == "session" -> "Whatever is running in it is ended."
    else -> "Every pane in it is closed, and whatever runs in them is ended."
}

@Composable
private fun AnchoredMenu(
    anchor: MenuAnchor,
    onDismiss: () -> Unit,
    content: @Composable ColumnScope.() -> Unit,
) {
    Box(Modifier.fillMaxSize()) {
        // A left click anywhere else closes it and opens nothing (#426) — caught, but not dimmed:
        // the sheet's scrim is right behind a 420 dp panel and wrong behind a 216 dp box that is
        // meant to read as hanging over the row it came from.
        Box(
            Modifier
                .fillMaxSize()
                .gestureAction("Close the menu", onClick = onDismiss)
                .clickable(remember { MutableInteractionSource() }, indication = null, onClick = onDismiss),
        )
        BoxWithConstraints {
            var size by remember { mutableStateOf(IntSize.Zero) }
            val tall = with(LocalDensity.current) { size.height.toDp() }
            val room = maxWidth
            val x = anchor.x.coerceIn(MENU_EDGE, (room - MENU_WIDTH - MENU_EDGE).coerceAtLeast(MENU_EDGE))
            // The box flips upward rather than off the bottom (#426). Zero on the first frame,
            // which places it at the pointer and corrects once it has been measured.
            val y =
                if (anchor.y + tall + MENU_EDGE > maxHeight) {
                    (anchor.y - tall).coerceAtLeast(MENU_EDGE)
                } else {
                    anchor.y
                }
            MenuPanel(Modifier.offset(x, y).onSizeChanged { size = it }.modal(onDismiss), content)
        }
    }
}

// What a pointer's menu looks like wherever it hangs: a narrow card of pointer-sized rows.
@Composable
fun MenuPanel(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Column(
        modifier
            .width(MENU_WIDTH)
            .background(tokens.color.bar, shape)
            .edge(tokens.chrome, shape)
            .padding(vertical = 5.dp),
    ) {
        CompositionLocalProvider(LocalSheetWide provides true) { content() }
    }
}
