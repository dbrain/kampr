package dev.kampr.shared.wire

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject

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
// `w` is columns per character in `x`, 1 or 2, omitted on the wire when 1. A double-width glyph
// occupies two columns (probe #210) and the client has no Unicode width table to work that out for
// itself, so the node says so; `x` is one code point per cell either way. `m` is what each cell of
// the run is wearing on top of its base —
// combining marks, ZWJ, variation selectors (probe #223) — by position, empty where a cell wears
// nothing and truncated after the last one. It rides beside `x` rather than in it so `x` stays one
// code point per cell and the row is still `sum(codepoints(x) * w)` columns wide.
data class Run(
    val s: Int = 0,
    val x: String = "",
    val l: Int? = null,
    val w: Int = 1,
    val m: List<String> = emptyList(),
)

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
    // Two nodes in one herd may be running different releases, and a client can only say so if
    // each node names its own; `detail` is why a node is offline, in the node's own words.
    val build: String? = null,
    // The release that supersedes `build`, named by the node it describes. Absent means there is
    // nothing to say — current, or it could not ask, or its operator turned the check off — and
    // all three render the same way, which is not at all.
    val update: String? = null,
    val detail: String? = null,
) {
    // A named session is its own herdr server and joins the herd as its own node, named
    // `<host>/<session>`; the primary session carries the bare host name.
    val host: String get() = name.substringBefore('/')
    val session: String get() = name.substringAfter('/', "default")
}

@Serializable
data class PaneInfo(
    val id: String,
    @SerialName("node_id") val nodeId: String,
    // A pane id carries its workspace but never its tab, so `tab.rename` and `tab.close` are only
    // addressable because the node sends the ids alongside the human-facing labels.
    @SerialName("workspace_id") val workspaceId: String? = null,
    @SerialName("tab_id") val tabId: String? = null,
    val workspace: String? = null,
    val tab: String? = null,
    val cwd: String? = null,
    val label: String? = null,
    // The harness's own name for the session, read off the marker it writes by pid. Generated, so
    // it loses to `label`, which is what the operator typed by hand — the naming template says so
    // and this only has to arrive.
    val title: String? = null,
    val agent: String? = null,
    @SerialName("agent_status") val agentStatus: String = "unknown",
    // Absent until something has measured the PTY: a soft wrap proves a width and the layout rect
    // does not, so an unwatched pane carries no width rather than a wrong one.
    val cols: Int? = null,
    val rows: Int = 24,
    @SerialName("scrollback_rows") val scrollbackRows: Int = 0,
    @SerialName("has_conversation") val hasConversation: Boolean = false,
    // Clients currently watching this pane, omitted when it is 0 or 1. A hub holds one watch for
    // every client behind it, so this can undercount a relayed pane and never overcounts.
    val watchers: Int? = null,
    @SerialName("updated_at") val updatedAt: String? = null,
    // Why this pane has no picture, in the node's own words — `NodeInfo.detail` one level down. A
    // node reaches herdr over a socket for the model and over a spawned binary for the screens and
    // can have exactly one of them working, which is a right herd and a blank grid for ever. Null
    // is the ordinary state; an empty pane is not a faulted one.
    val detail: String? = null,
    // The foreground job: `cmd` is its process name, `argv` its whole command line, and a pipeline
    // joins its members with ` | `. Absent far oftener than a schema reading suggests and never a
    // fault — a pane at its prompt has no job, and on a machine that sources ble.sh herdr names the
    // shell however busy the pane is (probe #297). `Naming`'s `[…]` is what drops the section.
    val cmd: String? = null,
    val argv: String? = null,
)

@Serializable
data class HerdDelta(
    val nodes: List<NodeInfo> = emptyList(),
    val panes: List<PaneInfo> = emptyList(),
)

@Serializable
data class PendingOption(val key: String, val label: String)

// The header of something a transcript mentions but never carries: a pasted screenshot is ~730 KB
// and the socket it would ride on is the one carrying live terminal frames, which it head-of-lines
// for seconds on a phone link. `id` is the handle for `GET /api/attachment/{pane}/{id}`. `kind` is
// an open string and anything unrecognised is a file offered as a download — that rule is what
// lets a node start producing a new kind without a client release.
@Serializable
data class Attachment(
    val id: String,
    val kind: String = "",
    val mime: String? = null,
    val bytes: Long? = null,
    val name: String? = null,
)

@Serializable(with = BlockSerializer::class)
sealed interface Block {
    data class Md(val text: String, val att: Attachment? = null) : Block
    data class Code(val lang: String?, val text: String) : Block
    data class Tool(val name: String, val summary: String?, val lines: Int?, val state: String?) : Block
    data class Diff(val path: String?, val text: String) : Block

    // A conversation this turn launched, offered for opening rather than spoken here. It rides
    // beside the tool card that started it, and its own turns are deliberately not inlined: a
    // client that rendered them here would be saying the pane's agent said what a subagent said.
    // `depth` is the node's word that a launched conversation can launch one of its own.
    data class Sub(
        val id: String,
        val kind: String? = null,
        val title: String? = null,
        val depth: Int? = null,
    ) : Block

    data class Unknown(val kind: String) : Block
}

// `kind` is what a turn is where that is not the same question as who filed it: `compact` is the
// harness's own summary of the conversation it dropped, filed under a `user` record with nothing
// but a flag to separate it from a prompt (#259). Additive and open — an unrecognised kind is a
// turn like any other, which is why it is not a third `role`.
@Serializable
data class Turn(
    val id: String,
    val role: String = "assistant",
    val at: String? = null,
    val blocks: List<Block> = emptyList(),
    val kind: String? = null,
)

// What the harness wrote down about the session, normalised across the three Kampr serves.
//
// **Every field is optional and only the ones something draws are modelled here.** The node fills
// a facet only where a harness has been *measured* to carry an equivalent, so one with nothing to
// say sends `{}`; and unknown fields are ignored by the wire's own rule, so reading a further
// facet later costs nothing but adding it here.
@Serializable
data class Facets(val queued: List<Queued> = emptyList())

// A prompt the operator has sent that the harness has not started on yet, and the enqueue stamp
// where it recorded one.
//
// The node folds this from the harness's own queue records rather than guessing at it — four
// operations, not the two an enqueue/remove pair suggests, because an ordinary delivery leaves a
// `dequeue` carrying no content at all and the head has to be taken by position (#320). So it is
// what the *harness* is waiting on and not what this client happens to have typed: a prompt sent
// from the desk, or from another phone, is in here too.
@Serializable
data class Queued(val text: String, val at: String? = null)

// `served` is whether this node reaches that session as a node of its own — a session can be
// running and unserved, and a pane on one will never appear in the herd. True by default: a node
// that says nothing, or a peer relayed through a hub on an older build, must not be drawn as
// unreachable.
@Serializable
data class SessionInfo(
    val name: String,
    val running: Boolean = false,
    val served: Boolean = true,
)

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

    // A device demoted or promoted while its socket is open. Not a second `hello`: that is
    // defined as the first message on a connection, and a client reading it as a greeting would
    // re-run everything a greeting means.
    data class RoleChanged(val role: String) : ServerMsg

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
        // This page is the pane's whole conversation, and anything else held for the pane belongs
        // to a transcript that has been left. Absent on the pages `convo.load` answers with —
        // older slices of the same transcript, which merge. Additive: a node that never sends it
        // gets the merging behaviour every build before it had.
        val fresh: Boolean = false,
        // Whose conversation this page is. Absent is the pane's own, which is every page any
        // build before this one ever saw; present names the handle a `sub` block carried, and a
        // page carrying it must never reach the pane's own turns.
        val sub: String? = null,
    ) : ServerMsg

    // `sub` names a launched conversation these turns belong to, absent for the pane's own. A
    // subagent's transcript grows while it runs, so what a reader opened keeps arriving instead of
    // going stale until they close and re-open it.
    data class ConvoTurn(val pane: String, val turns: List<Turn>, val sub: String? = null) : ServerMsg

    // What the harness wrote down about the *session* rather than about any one turn.
    //
    // **The newest one is the whole of it**, replacing whatever is held rather than merging into
    // it: the node folds the queue on the transcript's tail and republishes when it moves, and a
    // merge would leave a prompt the harness has already taken up standing for ever — which is
    // the defect the node's own fold was written to avoid (#320).
    data class ConvoFacets(val pane: String, val facets: Facets) : ServerMsg

    // The line the operator has half-typed at the pane's own keyboard and has not sent.
    //
    // **`input` appends to it.** Herdr's `pane.send_text` adds to whatever is already on the line,
    // so a sentence begun at the desk and a reply sent from here submit as one run-on line — and
    // until this frame existed nothing here had ever shown the first half of it.
    //
    // Not the live turn: that one is the message the *harness* is painting, this one is what a
    // *person* left in the box. `text == null` is an empty composer and takes the strip down, the
    // same shape `pending` uses for a question that has been answered.
    //
    // `clear` is the keystroke the node measured to empty *this harness's* composer, and the three
    // do not agree on it — so it is carried rather than looked up here, and a harness that sends
    // none is one whose takeover is not offered at all rather than guessed at.
    data class ConvoComposer(val pane: String, val text: String?, val clear: String?) : ServerMsg

    // question == null clears the prompt; there is no separate "resolved" message.
    data class Pending(
        val pane: String,
        val question: String?,
        val options: List<PendingOption>,
        val source: String,
    ) : ServerMsg

    data class Managed(
        val op: String,
        val ok: Boolean,
        val id: String?,
        val code: String? = null,
        val message: String? = null,
        val layout: JsonObject? = null,
    ) : ServerMsg

    data class NodeCaps(
        val node: String,
        val agentKinds: List<String>,
        val sessions: List<SessionInfo>,
    ) : ServerMsg

    // `node` is what the node is about when it is about a node rather than a pane, and it is the
    // only thing that lets this half tell a fault from an interruption: a node going unreachable
    // used to arrive with no subject at all, so every client showed it over whatever screen was
    // open and a node nobody was looking at interrupted a pane on a different one.
    data class Failure(
        val code: String,
        val message: String,
        val pane: String?,
        val node: String? = null,
    ) : ServerMsg

    data class Prefs(val panes: Map<String, PanePrefs>) : ServerMsg

    data class Pong(val n: Int) : ServerMsg

}

@Serializable
data class PanePrefs(val values: Map<String, String> = emptyMap()) {
    val zoom: Float? get() = values["zoom"]?.toFloatOrNull()
    val view: String? get() = values["view"]
    val confirm: Boolean get() = values["confirm"] != "off"
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

    // A page of a conversation this pane's agent launched. `id` is opaque and is only ever handed
    // back: it is minted by the node that served the turn and proved against that pane's own
    // session tree, so a client that built one would be asking for a file it cannot name.
    data class ConvoSub(val pane: String, val id: String, val before: String? = null) : ClientMsg

    // Bytes for the agent to work on. The node writes them to a file on the pane's own machine
    // and types the path in, because an agent reached over ssh reads a local path perfectly well
    // and it is the terminal's own image-paste protocol that dies. `name` is a hint at the stem
    // only — the node owns the directory and derives the extension from the bytes.
    data class Paste(val pane: String, val b64: String, val name: String? = null) : ClientMsg

    data class SetPrefs(val pane: String, val prefs: Map<String, String>) : ClientMsg

    data object Resync : ClientMsg

    data class Ping(val n: Int) : ClientMsg

    data class Manage(val request: ManageOp) : ClientMsg

    // Without this `caps.agent_kinds` and `caps.sessions` are dead on both ends: the node only
    // answers a `caps` it was asked for.
    data object RequestCaps : ClientMsg

}
