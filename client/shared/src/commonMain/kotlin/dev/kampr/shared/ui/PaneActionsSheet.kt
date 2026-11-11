package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
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
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.isNative
import dev.kampr.shared.model.nativeActions
import dev.kampr.shared.model.nativeFacts
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.model.workspaceKey
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.ZoomMode
import dev.kampr.shared.wire.isMuffit
import kotlinx.serialization.json.JsonObject

private sealed interface ActionStep {
    data class Rename(val kind: String, val name: String, val at: String, val clearable: Boolean) : ActionStep
    data class Close(val kind: String, val name: String, val holds: Int, val verb: String, val op: ManageOp) : ActionStep
    data class Focus(val kind: String, val name: String, val at: String) : ActionStep
}

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
    onFind: (() -> Unit)? = null,
    onFiles: (() -> Unit)? = null,
    tools: @Composable () -> Unit = {},
    desk: Boolean = false,
) {
    val compact = breakpoint != Breakpoint.Portrait
    var refusal by remember { mutableStateOf<String?>(null) }
    var step by remember(pane.id) { mutableStateOf<ActionStep?>(null) }
    // Herdr's split tree, held exactly as it was handed over: the client never reads inside it,
    // it just gives the same bytes back to `layout.apply`.
    var snapshot by remember(pane.tabId) { mutableStateOf<JsonObject?>(null) }

    LaunchedEffect(outcome) {
        val ack = outcome ?: return@LaunchedEffect
        when {
            !ack.ok -> refusal = ack.message ?: ack.code
            ack.op == "layout.export" -> snapshot = ack.layout
            ack.op in DISMISSING -> onDismiss()
        }
    }

    fun send(op: ManageOp) {
        refusal = null
        step = null
        onManage(op)
    }

    val back = { step = null }
    val scopes = paneScopes(pane, panes, snapshot, desk, ::send, { step = it }, onDismiss, onFind, onFiles, tools)
    BottomSheet(breakpoint, onDismiss, wideWidth = if (step == null && scopes.size > 1) SHEET_WIDE_WIDTH else DIALOG_WIDTH) {
        when (val at = step) {
            is ActionStep.Rename -> RenamePage(at.kind, if (at.at == pane.id) paneTitle(pane) else at.name, at.name, at.clearable, compact, back, onDismiss) { send(ManageOp.Rename(at.at, it)) }

            // Herdr counts the panes before it will close a thing that holds any — `Close
            // workspace?` over `bravo — 1 pane` (#426). This is the same count from the herd the
            // client already has.
            is ActionStep.Close -> {
                val verb = at.verb.replaceFirstChar(Char::uppercase)
                ConfirmPage(
                    title = "$verb this ${at.kind}?",
                    subject = held(at.kind, at.name, at.holds),
                    detail = closeDetail(at.verb, at.kind),
                    confirm = "$verb ${at.kind}",
                    compact = compact,
                    onBack = back,
                    onDismiss = onDismiss,
                    onConfirm = { send(at.op) },
                )
            }

            is ActionStep.Focus -> ConfirmPage(
                title = "Focus at the desk?",
                subject = at.name,
                detail = focusNote(at.kind),
                confirm = "Focus ${at.kind}",
                compact = compact,
                onBack = back,
                onDismiss = onDismiss,
                onConfirm = { send(ManageOp.Focus(at.at)) },
                danger = false,
            )

            null -> {
                SheetHeader(paneTitle(pane), null, null, onDismiss, compact, dismiss = "Close pane actions")
                Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState())) {
                    if (compact && scopes.size > 1) {
                        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            for (scope in scopes) Column(Modifier.weight(1f)) { scope() }
                        }
                    } else {
                        for (scope in scopes) scope()
                    }
                    if (pane.tabId == null && !pane.isNative && !pane.isMuffit) {
                        Note("This node does not send a tab id, so its tab cannot be addressed.")
                    }
                    Box(Modifier.height(14.dp))
                }
            }
        }
        Refusal(refusal)
    }
}

private typealias Scope = @Composable ColumnScope.() -> Unit

@Composable
private fun Note(text: String) {
    val tokens = Kampr.tokens
    KText(text, tokens.type.captionSmall, tokens.color.mute, Modifier.padding(horizontal = 20.dp, vertical = 4.dp), maxLines = 3)
}

// The pane (or session), its tab and its workspace: one group each, so which thing a command acts
// on is said once by the group rather than by every row.
private fun paneScopes(
    pane: PaneInfo,
    panes: List<PaneInfo>,
    snapshot: JsonObject?,
    desk: Boolean,
    send: (ManageOp) -> Unit,
    ask: (ActionStep) -> Unit,
    onDismiss: () -> Unit,
    onFind: (() -> Unit)?,
    onFiles: (() -> Unit)?,
    tools: @Composable () -> Unit,
): List<Scope> {
    val native = pane.isNative || pane.isMuffit
    val kind = if (native) "session" else "pane"
    val name = pane.label ?: pane.id.substringAfter('/')
    fun leaving(go: () -> Unit) = {
        onDismiss()
        go()
    }
    val own: Scope = {
        SheetSection("This $kind")
        onFind?.takeIf { !pane.isMuffit }?.let {
            // A search of the pane's *whole* scrollback, which is a different question from
            // anything on this screen: what a client holds is a window on the history, and the
            // node asks herdr for the rest (probe #511).
            MenuRow("Find in scrollback", leaving(it), KamprIcons.search, label = "Search this $kind's whole scrollback")
        }
        onFiles?.let {
            MenuRow("Browse files", leaving(it), KamprIcons.folder, label = "Browse the files where this $kind is working")
        }
        if (!pane.isMuffit) tools()
        if (native) {
            nativeActions(pane, desk).filterNot { it.confirm }.forEach { action ->
                MenuRow(action.verb.replaceFirstChar(Char::uppercase), { send(action.op) }, KamprIcons.desk, label = action.spoken)
            }
        } else {
            // Not "zoom". Kampr's own zoom is one screen away in the pane header, it magnifies the
            // rendered grid, and it is announced as "Zoom, currently 1.6x" — so a second control of
            // the same name is two unrelated things one tap apart. This one is herdr's
            // `pane.zoom`: the pane fills its tab on the machine and its siblings go away, which is
            // what the name says (probe #265).
            //
            // The label also names what it costs. herdr routes a zoom through focus before it works
            // out whether the zoom changes anything, and focus marks every pane in the tab seen —
            // so even a toggle that is a no-op destroys the operator's `done` marks for that whole
            // tab (probe #515).
            MenuRow(
                "Fill the tab",
                { send(ManageOp.PaneZoom(pane.id, ZoomMode.Toggle)) },
                KamprIcons.fill,
                label = "Make this pane fill its tab at the desk, and put the others back when it already does. " +
                    "Clears the unread marks for every pane in that tab.",
            )
            FocusRow("pane", name, pane.id, ask)
        }
        // The title is muffit's own, and docs/18 gives the node no way to rename a session.
        if (!pane.isMuffit) RenameRow(kind, name, pane.id, clearable = true, ask)
        val closing = if (native) nativeActions(pane, desk).firstOrNull { it.confirm } else null
        CloseRow(kind, name, 0, closing?.verb ?: "close", closing?.op ?: ManageOp.Close(pane.id), ask)
        val facts = if (native && !pane.isMuffit) nativeFacts(pane) else emptyList()
        if (facts.isNotEmpty()) Note(facts.joinToString(" · "))
    }
    if (pane.isMuffit) return listOf(own)
    // A Kampr session's tab and workspace are its node's own, so they close and rename as herdr's
    // do, and only herdr's can be put on the desk.
    val focusable = !native
    val scopes = mutableListOf(own)
    pane.tabId?.takeIf { focusable || panes.count { it.tabId == pane.tabId } > 1 }?.let { tabId ->
        val tab = pane.tab ?: tabId.substringAfter('/')
        val holds = panes.count { it.tabId == tabId }
        scopes.add({
            SheetSection("This tab · $tab")
            if (focusable) FocusRow("tab", tab, tabId, ask)
            MenuRow(
                "Remember the splits",
                { send(ManageOp.LayoutExport(tabId)) },
                KamprIcons.keep,
                label = "Remember how this tab is split, so it can be put back",
            )
            snapshot?.let { tree ->
                MenuRow(
                    "Put the splits back",
                    { send(ManageOp.LayoutApply(tabId, tree)) },
                    KamprIcons.restore,
                    label = "Split this tab the way it was when you remembered it",
                )
                Note("The remembered splits live on this device, and only until this sheet closes.")
            }
            if (focusable) RenameRow("tab", tab, tabId, clearable = false, ask)
            CloseRow("tab", tab, holds, "close", ManageOp.Close(tabId), ask)
        })
    }
    pane.workspaceKey?.let { workspaceId ->
        val workspace = pane.workspace ?: "this workspace"
        val holds = panes.count { it.workspaceKey == workspaceId }
        scopes.add({
            SheetSection("This workspace · $workspace")
            if (focusable) FocusRow("workspace", workspace, workspaceId, ask)
            RenameRow("workspace", workspace, workspaceId, clearable = false, ask)
            CloseRow("workspace", workspace, holds, "close", ManageOp.Close(workspaceId), ask)
        })
    }
    return scopes
}

// Focus is the one op that destroys herdr's `done` — the state it synthesises for a pane that
// finished while nobody was looking, which is the operator's unread flag (#357), and `tab.focus`
// and `workspace.focus` do it exactly as `pane.focus` does (#396). Every read leaves it standing,
// so this is the only control in Kampr that can take it away, and it says so before it does.
@Composable
private fun FocusRow(kind: String, name: String, at: String, ask: (ActionStep) -> Unit) {
    MenuRow("Focus at the desk…", { ask(ActionStep.Focus(kind, name, at)) }, KamprIcons.desk, label = "Focus this $kind on the machine's own screen")
}

@Composable
private fun RenameRow(kind: String, name: String, at: String, clearable: Boolean, ask: (ActionStep) -> Unit) {
    MenuRow("Rename $kind…", { ask(ActionStep.Rename(kind, name, at, clearable)) }, KamprIcons.pencil, label = "Rename this $kind")
}

@Composable
private fun CloseRow(kind: String, name: String, holds: Int, verb: String, op: ManageOp, ask: (ActionStep) -> Unit) {
    val title = verb.replaceFirstChar(Char::uppercase)
    MenuDivider()
    MenuRow(
        "$title $kind…",
        { ask(ActionStep.Close(kind, name, holds, verb, op)) },
        if (verb == "forget") KamprIcons.forget else KamprIcons.cross,
        danger = true,
        label = "$title this $kind",
    )
}

private val DISMISSING = setOf("close", "native.close", "native.forget")

private fun held(kind: String, name: String, holds: Int): String = when {
    kind == "pane" || kind == "session" || holds <= 0 -> name
    holds == 1 -> "$name — 1 pane"
    else -> "$name — $holds panes"
}

private fun focusNote(kind: String): String = if (kind == "pane") {
    "Puts this pane on the machine's own screen — and clears its done marker, the flag for an " +
        "agent that finished while nobody was looking."
} else {
    "Puts this $kind on the machine's own screen — and clears the done marker on whichever " +
        "pane it lands on, the flag for an agent that finished while nobody was looking."
}
