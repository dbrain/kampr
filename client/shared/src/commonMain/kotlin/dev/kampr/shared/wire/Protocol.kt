package dev.kampr.shared.wire

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable(with = ColorSpecSerializer::class)
sealed interface ColorSpec {
    data object Default : ColorSpec
    data class Indexed(val v: Int) : ColorSpec
    data class Rgb(val r: Int, val g: Int, val b: Int) : ColorSpec
}

@Serializable
data class Style(
    val fg: ColorSpec = ColorSpec.Default,
    val bg: ColorSpec = ColorSpec.Default,
    val bold: Boolean = false,
    val dim: Boolean = false,
    val italic: Boolean = false,
    val underline: Boolean = false,
    val blink: Boolean = false,
    val reverse: Boolean = false,
    val strike: Boolean = false,
    val hidden: Boolean = false,
)

@Serializable
data class Run(val s: Int = 0, val x: String = "", val l: Int? = null)

@Serializable
data class RowDiff(val row: Int, val runs: List<Run> = emptyList())

@Serializable
data class Cursor(val col: Int = 0, val row: Int = 0, val visible: Boolean = true)

@Serializable
data class Caps(
    val push: Boolean = false,
    val scrollback: Boolean = false,
    val conversation: Boolean = false,
    val manage: Boolean = false,
)

// What the client may offer is decided here, never by inspecting the URL: an affordance that
// cannot work must be absent rather than present-and-failing.
@Serializable
data class Security(
    val tier: Int = 0,
    val encrypted: Boolean = false,
    @SerialName("unencrypted_banner") val unencryptedBanner: Boolean = true,
    val passkeys: Boolean = false,
    val push: Boolean = false,
    val installable: Boolean = false,
    val unlocks: List<String> = emptyList(),
)

@Serializable
data class NodeInfo(
    val id: String,
    val name: String = id,
    val kind: String = "peer",
    val online: Boolean = true,
    @SerialName("rtt_ms") val rttMs: Double? = null,
    @SerialName("herdr_version") val herdrVersion: String? = null,
)

@Serializable
data class PaneInfo(
    val id: String,
    @SerialName("node_id") val nodeId: String,
    val workspace: String? = null,
    val tab: String? = null,
    val cwd: String? = null,
    val label: String? = null,
    val agent: String? = null,
    @SerialName("agent_status") val agentStatus: String = "unknown",
    val cols: Int = 80,
    val rows: Int = 24,
    @SerialName("scrollback_rows") val scrollbackRows: Int = 0,
    @SerialName("has_conversation") val hasConversation: Boolean = false,
    @SerialName("updated_at") val updatedAt: String? = null,
)

@Serializable
data class HerdDelta(
    val nodes: List<NodeInfo> = emptyList(),
    val panes: List<PaneInfo> = emptyList(),
)

@Serializable
data class PendingOption(val key: String, val label: String)

@Serializable(with = BlockSerializer::class)
sealed interface Block {
    data class Md(val text: String) : Block
    data class Code(val lang: String?, val text: String) : Block
    data class Tool(val name: String, val summary: String?, val lines: Int?, val state: String?) : Block
    data class Diff(val path: String?, val text: String) : Block
    data class Unknown(val kind: String) : Block
}

@Serializable
data class Turn(
    val id: String,
    val role: String = "assistant",
    val at: String? = null,
    val blocks: List<Block> = emptyList(),
)

@Serializable
data class SessionInfo(val name: String, val running: Boolean = false)

sealed interface ServerMsg {
    data class Hello(
        val protocol: Int,
        val nodeId: String,
        val nodeName: String,
        val build: String,
        val role: String,
        val caps: Caps,
        val security: Security,
    ) : ServerMsg

    data class Herd(val nodes: List<NodeInfo>, val panes: List<PaneInfo>) : ServerMsg

    data class HerdPatch(
        val added: HerdDelta,
        val changed: HerdDelta,
        val removedIds: List<String>,
    ) : ServerMsg

    data class Styles(val from: Int, val styles: List<Style>) : ServerMsg

    data class GridReset(
        val pane: String,
        val cols: Int,
        val rows: Int,
        val rowsData: List<RowDiff>,
        val cursor: Cursor,
        val links: List<String>,
    ) : ServerMsg

    data class GridPatch(
        val pane: String,
        val rows: List<RowDiff>,
        val cursor: Cursor?,
        val links: List<String>,
    ) : ServerMsg

    data class Scrollback(
        val pane: String,
        val fromTop: Int,
        val rows: List<RowDiff>,
        val totalRows: Int,
        val complete: Boolean,
        val capped: Boolean,
    ) : ServerMsg

    data class Convo(
        val pane: String,
        val cursor: String?,
        val more: Boolean,
        val turns: List<Turn>,
    ) : ServerMsg

    data class ConvoTurn(val pane: String, val turns: List<Turn>) : ServerMsg

    // question == null clears the prompt; there is no separate "resolved" message.
    data class Pending(
        val pane: String,
        val question: String?,
        val options: List<PendingOption>,
        val source: String,
    ) : ServerMsg

    data class Managed(val op: String, val ok: Boolean, val id: String?) : ServerMsg

    data class NodeCaps(
        val node: String,
        val agentKinds: List<String>,
        val sessions: List<SessionInfo>,
    ) : ServerMsg

    data class Failure(val code: String, val message: String, val pane: String?) : ServerMsg

    data class Prefs(val panes: Map<String, PanePrefs>) : ServerMsg

    data class Pong(val n: Int) : ServerMsg
}

@Serializable
data class PanePrefs(val values: Map<String, String> = emptyMap()) {
    val zoom: Float? get() = values["zoom"]?.toFloatOrNull()
    val view: String? get() = values["view"]
}

sealed interface ClientMsg {
    data class Watch(
        val pane: String,
        val scrollback: Boolean = true,
        val conversation: Boolean = true,
    ) : ClientMsg

    data class Unwatch(val pane: String) : ClientMsg

    data class InputText(val pane: String, val text: String) : ClientMsg

    data class InputB64(val pane: String, val b64: String) : ClientMsg

    data class InputKeys(val pane: String, val keys: List<String>) : ClientMsg

    data class Answer(val pane: String, val key: String) : ClientMsg

    data class ConvoLoad(val pane: String, val before: String?) : ClientMsg

    data class SetPrefs(val pane: String, val prefs: Map<String, String>) : ClientMsg

    data object Resync : ClientMsg

    data class Ping(val n: Int) : ClientMsg

    data class Manage(val op: String, val fields: Map<String, String?>) : ClientMsg
}
