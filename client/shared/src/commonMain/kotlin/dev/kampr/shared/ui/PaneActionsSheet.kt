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
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.ZoomMode
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

    LaunchedEffect(outcome) {
        val ack = outcome ?: return@LaunchedEffect
        if (!ack.ok) refusal = ack.message ?: ack.code else if (ack.op == "close") onDismiss()
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
                    Target("tab", pane.tab ?: tabId.substringAfter('/'), tabId, clearable = false, zoomable = false, onManage = ::send)
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
                refusal?.let { KText(it, tokens.type.captionSmall, tokens.color.blocked, maxLines = 3) }
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
            Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                LabelText(kind, tokens.type.micro, tokens.color.mute, Modifier.weight(1f))
                KText(name, tokens.type.meta, tokens.color.dim)
            }
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Chip("focus", false, { onManage(ManageOp.Focus(at)) })
                if (zoomable) Chip("zoom", false, { onManage(ManageOp.PaneZoom(at, ZoomMode.Toggle)) })
                Chip("rename", renaming, { renaming = !renaming })
                Chip("close", confirming, { confirming = !confirming }, quiet = !confirming)
            }
            if (renaming) {
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
                ) {
                    KField("label", label, Modifier.weight(1f)) { label = it }
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
                    verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
                ) {
                    KText(
                        if (kind == "tab") "Closes every pane in it." else "Close $name?",
                        tokens.type.captionSmall,
                        tokens.color.blocked,
                        Modifier.weight(1f),
                        maxLines = 2,
                    )
                    Chip("cancel", false, { confirming = false }, quiet = true)
                    Chip("close", true, { confirming = false; onManage(ManageOp.Close(at)) })
                }
            }
        }
    }
}
