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

@Composable
fun PaneActionsSheet(
    breakpoint: Breakpoint,
    pane: PaneInfo,
    outcome: ServerMsg.Managed?,
    onManage: (ManageOp) -> Unit,
    onDismiss: () -> Unit,
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
                        onManage = ::send,
                    ) {
                        Chip(
                            "snapshot shape", false, { send(ManageOp.LayoutExport(tabId)) },
                            label = "Snapshot the split shape of this tab",
                        )
                        snapshot?.let { tree ->
                            Chip(
                                "put it back", false, { send(ManageOp.LayoutApply(tabId, tree)) },
                                label = "Restore the snapshotted split shape",
                            )
                        }
                    }
                }
                Target(
                    kind = "workspace",
                    name = pane.workspace ?: "this workspace",
                    at = pane.workspaceId ?: workspaceIdOf(pane.id),
                    clearable = false,
                    zoomable = false,
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

@Composable
private fun Target(
    kind: String,
    name: String,
    at: String,
    clearable: Boolean,
    zoomable: Boolean,
    onManage: (ManageOp) -> Unit,
    extra: @Composable (() -> Unit)? = null,
) {
    val tokens = Kampr.tokens
    var renaming by remember { mutableStateOf(false) }
    var confirming by remember { mutableStateOf(false) }
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
                Chip("focus", false, { onManage(ManageOp.PaneZoom(at, ZoomMode.Toggle)) }, label = "Focus this $kind at the desk")
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
                        "An empty label clears it back to the pane's own name.",
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
                    KText(
                        if (kind == "tab") "Closes every pane in it." else "Close $name?",
                        tokens.type.captionSmall,
                        tokens.color.blocked,
                        Modifier.weight(1f).announce(
                            if (kind == "tab") "Closing this tab closes every pane in it. Confirm or cancel."
                            else "Close $name? Confirm or cancel.",
                            urgent = true,
                        ),
                        maxLines = 2,
                    )
                    Chip("cancel", false, { confirming = false }, quiet = true, label = "Do not close it")
                    Chip(
                        "close", true, { confirming = false; onManage(ManageOp.Close(at)) },
                        label = if (kind == "tab") "Confirm — close the tab and every pane in it"
                        else "Confirm — close $name",
                    )
                }
            }
        }
    }
}
