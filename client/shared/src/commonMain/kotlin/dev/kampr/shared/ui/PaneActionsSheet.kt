package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.ZoomMode
import kotlinx.serialization.json.JsonObject
import dev.kampr.shared.wire.workspaceIdOf

// The in-session sheet, which is the half of this that may talk about the desk at all: it is
// reached from the pane's own header and from a mosaic cell, so the operator is looking at the
// screen every control here reshapes. The list menu (`PaneMenu`) carries none of it.
@Composable
fun PaneActionsSheet(
    breakpoint: Breakpoint,
    pane: PaneInfo,
    outcome: ServerMsg.Managed?,
    onManage: (ManageOp) -> Unit,
    onDismiss: () -> Unit,
    panes: List<PaneInfo> = emptyList(),
) {
    val tokens = Kampr.tokens
    var refusal by remember { mutableStateOf<String?>(null) }
    // Herdr's split tree, held exactly as it was handed over: the client never reads inside it,
    // it just gives the same bytes back to `layout.apply`.
    var snapshot by remember(pane.tabId) { mutableStateOf<JsonObject?>(null) }

    LaunchedEffect(outcome) {
        val ack = outcome ?: return@LaunchedEffect
        when {
            !ack.ok -> refusal = ack.message ?: ack.code
            ack.op == "layout.export" -> snapshot = ack.layout
            ack.op == "close" -> onDismiss()
        }
    }

    fun send(op: ManageOp) {
        refusal = null
        onManage(op)
    }

    BottomSheet(breakpoint, onDismiss) {
        SheetHeader("Actions", paneTitle(pane), null, onDismiss)
        Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState())) {
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(9.dp)) {
                Target(
                    kind = "pane",
                    name = pane.label ?: pane.id.substringAfter('/'),
                    at = pane.id,
                    // Only a pane's label is nullable — herdr's tab and workspace rename take a
                    // required string, so there is nothing to clear them to.
                    clearable = true,
                    zoomable = true,
                    onManage = ::send,
                )
                pane.tabId?.let { tabId ->
                    Target(
                        kind = "tab",
                        name = pane.tab ?: tabId.substringAfter('/'),
                        at = tabId,
                        clearable = false,
                        zoomable = false,
                        holds = panes.count { it.tabId == tabId },
                        onManage = ::send,
                    ) {
                        Chip(
                            "remember the splits", false, { send(ManageOp.LayoutExport(tabId)) },
                            label = "Remember how this tab is split, so it can be put back",
                        )
                        snapshot?.let { tree ->
                            Chip(
                                "put the splits back", false, { send(ManageOp.LayoutApply(tabId, tree)) },
                                label = "Split this tab the way it was when you remembered it",
                            )
                        }
                    }
                    if (snapshot != null) {
                        KText(
                            "The remembered splits live on this device, and only until this sheet closes.",
                            tokens.type.captionSmall,
                            tokens.color.mute,
                            maxLines = 2,
                        )
                    }
                }
                val workspaceId = pane.workspaceId ?: workspaceIdOf(pane.id)
                Target(
                    kind = "workspace",
                    name = pane.workspace ?: "this workspace",
                    at = workspaceId,
                    clearable = false,
                    zoomable = false,
                    holds = panes.count { (it.workspaceId ?: workspaceIdOf(it.id)) == workspaceId },
                    onManage = ::send,
                )
                if (pane.tabId == null) {
                    KText(
                        "This node does not send a tab id, so its tab cannot be addressed.",
                        tokens.type.captionSmall,
                        tokens.color.mute,
                        maxLines = 2,
                    )
                }
                refusal?.let {
                    KText(it, tokens.type.captionSmall, tokens.color.blocked, Modifier.announce(it, urgent = true), maxLines = 3)
                }
            }
            Box(Modifier.height(18.dp))
        }
    }
}

// Herdr counts the panes before it will close a thing that holds any — `Close workspace?` over
// `bravo — 1 pane` (#426). This is the same count from the herd the client already has.
private fun held(kind: String, name: String, holds: Int): String =
    when {
        kind == "pane" || holds <= 0 -> name
        holds == 1 -> "$name — 1 pane"
        else -> "$name — $holds panes"
    }

// Focus is the one op that destroys herdr's `done` — the state it synthesises for a pane that
// finished while nobody was looking, which is the operator's unread flag (#357), and `tab.focus`
// and `workspace.focus` do it exactly as `pane.focus` does (#396). Every read leaves it standing,
// so this is the only control in Kampr that can take it away, and it says so before it does.
private fun focusNote(kind: String): String =
    if (kind == "pane") {
        "Puts this pane on the machine's own screen — and clears its done marker, the flag for an " +
            "agent that finished while nobody was looking."
    } else {
        "Puts this $kind on the machine's own screen — and clears the done marker on whichever " +
            "pane it lands on, the flag for an agent that finished while nobody was looking."
    }

@Composable
private fun Target(
    kind: String,
    name: String,
    at: String,
    clearable: Boolean,
    zoomable: Boolean,
    onManage: (ManageOp) -> Unit,
    holds: Int = 0,
    extra: @Composable (() -> Unit)? = null,
) {
    val tokens = Kampr.tokens
    var renaming by remember { mutableStateOf(false) }
    var confirming by remember { mutableStateOf(false) }
    var focusing by remember { mutableStateOf(false) }
    var label by remember(at) { mutableStateOf(TextFieldValue(name)) }

    Surface(Modifier.fillMaxWidth()) {
        Column(
            Modifier.padding(horizontal = 15.dp, vertical = 13.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(Modifier.named("$kind: $name"), verticalAlignment = Alignment.CenterVertically) {
                LabelText(kind, tokens.type.micro, tokens.color.mute, Modifier.weight(1f))
                KText(name, tokens.type.meta, tokens.color.dim)
            }
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Chip(
                    "focus at the desk", focusing, { focusing = !focusing },
                    label = "Focus this $kind on the machine's own screen",
                )
                if (zoomable) {
                    // Not "zoom". Kampr's own zoom is one screen away in the pane header, it
                    // magnifies the rendered grid, and it is announced as "Zoom, currently 1.6x" —
                    // so a second control of the same name is two unrelated things one tap apart,
                    // and "at the desk" was carrying the entire distinction. This one is herdr's
                    // `pane.zoom`: the pane fills its tab on the machine and its siblings go away,
                    // which is what the name now says (probe #265).
                    Chip(
                        "fill the tab", false, { onManage(ManageOp.PaneZoom(at, ZoomMode.Toggle)) },
                        label = "Make this $kind fill its tab at the desk, and put the others back when it already does",
                    )
                }
                Chip("rename", renaming, { renaming = !renaming }, label = "Rename this $kind", )
                Chip(
                    "close", confirming, { confirming = !confirming }, quiet = !confirming,
                    label = "Close this $kind",
                )
                extra?.invoke()
            }
            if (focusing) {
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    KText(
                        focusNote(kind),
                        tokens.type.captionSmall,
                        tokens.color.mute,
                        Modifier.weight(1f).announce(focusNote(kind), urgent = true),
                        maxLines = 3,
                    )
                    Chip("cancel", false, { focusing = false }, quiet = true, label = "Leave the desk alone")
                    Chip(
                        "focus", true, { focusing = false; onManage(ManageOp.Focus(at)) },
                        label = "Confirm — focus $name at the desk and clear its done marker",
                    )
                }
            }
            if (renaming) {
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    KField("label", label, Modifier.weight(1f), label = "New name for this $kind") { label = it }
                    Chip(
                        "save",
                        false,
                        {
                            val next = label.text.trim().ifEmpty { null }
                            if (next != null || clearable) {
                                onManage(ManageOp.Rename(at, next))
                                renaming = false
                            }
                        },
                        label = "Save the new name",
                    )
                }
                if (clearable) {
                    KText(
                        "An empty name puts back what the pane calls itself.",
                        tokens.type.captionSmall,
                        tokens.color.mute,
                        maxLines = 2,
                    )
                }
            }
            if (confirming) {
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f).announce("Close $kind? ${held(kind, name, holds)}. Confirm or cancel.", urgent = true)) {
                        KText("Close $kind?", tokens.type.captionSmall, tokens.color.blocked, maxLines = 1)
                        KText(held(kind, name, holds), tokens.type.captionSmall, tokens.color.mute, maxLines = 2)
                    }
                    Chip("cancel", false, { confirming = false }, quiet = true, label = "Do not close it")
                    Chip(
                        "close", true, { confirming = false; onManage(ManageOp.Close(at)) },
                        label = "Confirm — close ${held(kind, name, holds)}",
                    )
                }
            }
        }
    }
}
