package dev.kampr.shared.wire

import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

enum class SplitDirection(val wire: String) { Right("right"), Down("down") }

enum class ZoomMode(val wire: String) { Toggle("toggle"), On("on"), Off("off") }

// `Once` resizes and hands the PTY straight back; `Hold` keeps the claim so the size survives on a
// pane a desk is attached to, which restores its own geometry the moment a controller lets go;
// `Release` gives a hold up. Off by default everywhere — a held pane is one the desk cannot reshape.
enum class SizeMode(val wire: String) {
    Once("once"),
    Hold("hold"),
    // `Hold` with an owner and an undo: the node ties it to this websocket, so it ends when
    // the socket does however it ends, and letting go puts the pane back (ADR 0013).
    Match("match"),
    Release("release"),
}

// The smallest pane the node will produce, mirrored from `crates/kampr-node/src/manage.rs` so the
// client can grey a control out rather than offer one the node refuses. A resize on a headless pane
// persists, so fitting a pane to a small screen would leave it that narrow for every other client.
const val MIN_PANE_COLS = 80
const val MIN_PANE_ROWS = 24

// One type per op, because `Map<String, String?>` could express four of them: `ratio` is a float,
// `env` an object, `args` an array and `layout` a nested tree. Field names and shapes here are the
// ones `crates/kampr-node/src/manage.rs` deserialises.
sealed interface ManageOp {
    val op: String

    data class WorkspaceCreate(
        val node: String,
        val label: String? = null,
        val cwd: String? = null,
        val env: Map<String, String> = emptyMap(),
    ) : ManageOp {
        override val op: String get() = "workspace.create"
    }

    // `at` is a workspace id. The node accepts a tab or pane id too and derives the workspace.
    data class TabCreate(
        val at: String,
        val label: String? = null,
        val cwd: String? = null,
    ) : ManageOp {
        override val op: String get() = "tab.create"
    }

    data class PaneSplit(
        val at: String,
        val direction: SplitDirection,
        val ratio: Double? = null,
        val cwd: String? = null,
    ) : ManageOp {
        override val op: String get() = "pane.split"
    }

    data class PaneZoom(val at: String, val mode: ZoomMode = ZoomMode.Toggle) : ManageOp {
        override val op: String get() = "pane.zoom"
    }

    // The one op that reshapes a pane. `cols`/`rows` are absent on a `Release`, which names no
    // size because it is only letting go of one.
    data class PaneSize(
        val at: String,
        val cols: Int? = null,
        val rows: Int? = null,
        val mode: SizeMode = SizeMode.Once,
    ) : ManageOp {
        override val op: String get() = "pane.size"
    }

    // A null label clears a pane's; a tab or a workspace refuses it with bad_request.
    data class Rename(val at: String, val label: String?) : ManageOp {
        override val op: String get() = "rename"
    }

    data class Close(val at: String) : ManageOp {
        override val op: String get() = "close"
    }

    data class Focus(val at: String) : ManageOp {
        override val op: String get() = "focus"
    }

    data class AgentStart(
        val at: String,
        val kind: String,
        val name: String? = null,
        val args: List<String> = emptyList(),
    ) : ManageOp {
        override val op: String get() = "agent.start"
    }

    data class WorktreeCreate(
        val node: String,
        val branch: String,
        val base: String? = null,
        val cwd: String? = null,
        val label: String? = null,
    ) : ManageOp {
        override val op: String get() = "worktree.create"
    }

    data class WorktreeOpen(
        val node: String,
        val path: String,
        val cwd: String? = null,
        val label: String? = null,
    ) : ManageOp {
        override val op: String get() = "worktree.open"
    }

    data class LayoutExport(val at: String) : ManageOp {
        override val op: String get() = "layout.export"
    }

    // Herdr's own split tree, opaque here: an export handed back unchanged, or just its root.
    data class LayoutApply(val at: String, val layout: JsonObject) : ManageOp {
        override val op: String get() = "layout.apply"
    }

    data class SessionCreate(val node: String, val name: String) : ManageOp {
        override val op: String get() = "session.create"
    }

    data class SessionStop(val node: String, val name: String) : ManageOp {
        override val op: String get() = "session.stop"
    }

    // One command on one host, as part of a fan-out. `cohort` is assigned by the client because a
    // run spans hosts and no single node can name one.
    //
    // `command` is the line the operator typed, handed to the host's own shell, so `&&`, `|`, `;`,
    // quotes and globs mean there what they mean in their terminal. It is **additive beside**
    // `args`, which older clients still send and which is still an argv `exec`ed with nothing in
    // front of it — a node that receives both takes `command`, because only a client that knows
    // about it sends it. A node too old to know the field answers that `fleet.run` needs `args`,
    // which is a refusal the operator can read rather than a command half-run.
    data class FleetRun(
        val node: String,
        val cohort: String,
        val command: String,
        val cwd: String? = null,
    ) : ManageOp {
        override val op: String get() = "fleet.run"
    }

    data class FleetStop(val at: String) : ManageOp {
        override val op: String get() = "fleet.stop"
    }

    // Drops a *finished* run from the board. The node refuses a live one — forgetting it would
    // leave nothing reading its pty and nobody able to answer it.
    data class FleetForget(val at: String) : ManageOp {
        override val op: String get() = "fleet.forget"
    }

    // Keeps a command in the node's fleet book: by `entry` when the operator pressed one already
    // in the book, by `args` when they typed it. Promotion by id rather than by argv because
    // re-deriving "the same command" from a re-typed line is a second chance to disagree, and a
    // disagreement here means the command in the history *and* in Saved.
    //
    // Nothing about it runs anything. A saved command is staged into the run sheet and fires
    // through the same confirmation a typed one does — one press across the whole herd should not
    // be cheaper than typing it.
    data class FleetSave(
        val entry: String? = null,
        val args: List<String> = emptyList(),
        val cwd: String? = null,
        val label: String? = null,
    ) : ManageOp {
        override val op: String get() = "fleet.save"
    }

    // Removes one book entry, history or saved. The rule that keeps credentials out of the
    // automatic half is a reduction rather than a filter, so this is the part that actually holds.
    data class FleetDrop(val entry: String) : ManageOp {
        override val op: String get() = "fleet.drop"
    }
}

fun ManageOp.fields(): JsonObject = buildJsonObject {
    fun opt(key: String, value: String?) {
        if (value != null) put(key, value)
    }
    when (this@fields) {
        is ManageOp.WorkspaceCreate -> {
            put("node", node)
            opt("label", label)
            opt("cwd", cwd)
            if (env.isNotEmpty()) {
                put("env", buildJsonObject { env.forEach { (k, v) -> put(k, v) } })
            }
        }
        is ManageOp.TabCreate -> {
            put("at", at)
            opt("label", label)
            opt("cwd", cwd)
        }
        is ManageOp.PaneSplit -> {
            put("at", at)
            put("direction", direction.wire)
            if (ratio != null) put("ratio", JsonPrimitive(ratio))
            opt("cwd", cwd)
        }
        is ManageOp.PaneZoom -> {
            put("at", at)
            put("mode", mode.wire)
        }
        is ManageOp.PaneSize -> {
            put("at", at)
            if (cols != null) put("cols", JsonPrimitive(cols))
            if (rows != null) put("rows", JsonPrimitive(rows))
            // Omitted when it is the default, matching the fixture's plain `pane.size` case.
            if (mode != SizeMode.Once) put("mode", mode.wire)
        }
        is ManageOp.Rename -> {
            put("at", at)
            put("label", label?.let(::JsonPrimitive) ?: JsonNull)
        }
        is ManageOp.Close -> put("at", at)
        is ManageOp.Focus -> put("at", at)
        is ManageOp.AgentStart -> {
            put("at", at)
            put("kind", kind)
            opt("name", name)
            put("args", buildJsonArray { args.forEach { add(JsonPrimitive(it)) } })
        }
        is ManageOp.WorktreeCreate -> {
            put("node", node)
            put("branch", branch)
            opt("base", base)
            opt("cwd", cwd)
            opt("label", label)
        }
        is ManageOp.WorktreeOpen -> {
            put("node", node)
            put("path", path)
            opt("cwd", cwd)
            opt("label", label)
        }
        is ManageOp.LayoutExport -> put("at", at)
        is ManageOp.LayoutApply -> {
            put("at", at)
            put("layout", layout)
        }
        is ManageOp.SessionCreate -> {
            put("node", node)
            put("name", name)
        }
        is ManageOp.FleetRun -> {
            put("node", node)
            put("cohort", cohort)
            put("command", command)
            opt("cwd", cwd)
        }
        is ManageOp.FleetStop -> put("at", at)
        is ManageOp.FleetForget -> put("at", at)
        is ManageOp.FleetSave -> {
            // `entry` and not `at`: `at` is routed, and a book entry names no host, so putting one
            // there would send this op down a mesh link looking for the node that owns it.
            opt("entry", entry)
            if (args.isNotEmpty()) put("args", buildJsonArray { args.forEach { add(JsonPrimitive(it)) } })
            opt("cwd", cwd)
            opt("label", label)
        }
        is ManageOp.FleetDrop -> put("entry", entry)
        is ManageOp.SessionStop -> {
            put("node", node)
            put("name", name)
        }
    }
}

// Herdr's id grammar, which the node re-derives on every op: `w3` is a workspace, `w3:t1` a tab,
// `w3:p2` a pane, all prefixed with the node id.
fun workspaceIdOf(paneId: String): String {
    val node = paneId.substringBefore('/')
    val local = paneId.substringAfter('/').substringBefore(':')
    return "$node/$local"
}
