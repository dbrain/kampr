package dev.kampr.shared.model

import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NativeExit
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.isMuffit

private const val NATIVE = "native"

val PaneInfo.isNative: Boolean get() = provider == NATIVE

// `native.desk` is the one native op that needs herdr, and the node starts a cold one for it, so a
// machine that has ever named its herdr version can host a session as well as one whose herdr is up.
val NodeInfo.hasDesk: Boolean get() = online || herdrVersion != null

fun NodeInfo.serves(pane: PaneInfo?): Boolean = if (pane?.isNative == true || pane?.isMuffit == true) isReachable else online

fun exitPhrase(exit: NativeExit): String = when {
    exit.signal != null -> "killed by signal ${exit.signal}"
    exit.code != null -> "exited ${exit.code}"
    else -> "exited"
}

fun nativeFacts(pane: PaneInfo): List<String> {
    if (pane.isMuffit) return listOf("muffit")
    if (!pane.isNative) return emptyList()
    val exit = pane.exited
    return listOfNotNull(
        "kampr",
        "not on desk".takeIf { pane.detached && exit == null },
        exit?.let(::exitPhrase),
    )
}

fun nativeSpoken(pane: PaneInfo): String? = nativeFacts(pane).takeIf { it.isNotEmpty() }?.let { facts ->
    (listOf(if (pane.isMuffit) "Muffit session" else "Kampr session") + facts.drop(1)).joinToString(", ")
}

// `confirm` is whether the press only asks: closing hangs the program up and forgetting deletes its
// log, and neither can be taken back. Showing it on the desk adds a herdr pane and loses nothing.
data class NativeAction(val verb: String, val spoken: String, val op: ManageOp, val confirm: Boolean)

// The node maps `native.close` on a muffit session to muffit's own stop.
fun nativeActions(pane: PaneInfo, desk: Boolean): List<NativeAction> {
    val title = paneTitle(pane)
    if (pane.isMuffit) return listOf(NativeAction("close", "Close $title", ManageOp.NativeClose(pane.id), true))
    if (!pane.isNative) return emptyList()
    if (pane.exited != null) {
        return listOf(NativeAction("forget", "Forget $title and its output", ManageOp.NativeForget(pane.id), true))
    }
    return listOfNotNull(
        NativeAction("show on desk", "Show $title on the desk", ManageOp.NativeDesk(pane.id), false)
            .takeIf { pane.detached && desk },
        NativeAction("close", "Close $title", ManageOp.NativeClose(pane.id), true),
    )
}
