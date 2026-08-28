package dev.kampr.shared

import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.SplitDirection
import dev.kampr.shared.wire.Wire
import dev.kampr.shared.wire.SizeMode
import dev.kampr.shared.wire.ZoomMode
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.fail

// `crates/kampr-node/tests/manage_wire.rs` asserts the node deserialises this exact file. Every
// seam defect in this project came from both sides agreeing with each other and neither agreeing
// with the wire, so neither side owns the fixture.
private const val FIXTURE = "crates/kampr-node/tests/fixtures/manage-ops.json"

class ManageWireTest {
    private val cases: Map<String, JsonObject> by lazy {
        var dir = File(".").absoluteFile
        repeat(5) {
            val candidate = File(dir, FIXTURE)
            if (candidate.isFile) {
                return@lazy (Wire.json.parseToJsonElement(candidate.readText()) as JsonObject)
                    .mapValues { (_, v) -> v as JsonObject }
            }
            dir = dir.parentFile ?: return@repeat
        }
        fail("could not find $FIXTURE from ${File(".").absolutePath}")
    }

    private val ops: Map<String, ManageOp> = mapOf(
        "workspace.create" to ManageOp.WorkspaceCreate(
            node = "01JNODE",
            label = "kampr",
            cwd = "/home/dbrain/dev/kampr",
            env = mapOf("RUST_LOG" to "debug", "KAMPR_ENV" to "probe"),
        ),
        "workspace.create.bare" to ManageOp.WorkspaceCreate(node = "01JNODE"),
        "tab.create" to ManageOp.TabCreate("01JNODE/w3", "tests", "/home/dbrain/dev/kampr"),
        "pane.split" to ManageOp.PaneSplit("01JNODE/w3:p2", SplitDirection.Right, 0.35),
        "pane.zoom" to ManageOp.PaneZoom("01JNODE/w3:p2", ZoomMode.Toggle),
        "pane.size" to ManageOp.PaneSize("01JNODE/w3:p2", cols = 200, rows = 50),
        "pane.size.hold" to ManageOp.PaneSize(
            at = "01JNODE/w3:p2",
            cols = 200,
            rows = 50,
            mode = SizeMode.Hold,
        ),
        "pane.size.release" to ManageOp.PaneSize("01JNODE/w3:p2", mode = SizeMode.Release),
        "rename" to ManageOp.Rename("01JNODE/w3:p2", "build"),
        "rename.clear" to ManageOp.Rename("01JNODE/w3:p2", null),
        "close" to ManageOp.Close("01JNODE/w3:t1"),
        "focus" to ManageOp.Focus("01JNODE/w3"),
        "agent.start" to ManageOp.AgentStart(
            at = "01JNODE/w3:p2",
            kind = "claude",
            name = "reviewer",
            args = listOf("--model", "opus"),
        ),
        "worktree.create" to ManageOp.WorktreeCreate(
            node = "01JNODE",
            branch = "feat/mesh-auth",
            base = "main",
            cwd = "/home/dbrain/dev/kampr",
            label = "mesh-auth",
        ),
        "worktree.open" to ManageOp.WorktreeOpen("01JNODE", "/home/dbrain/dev/kampr-feat-x"),
        "layout.export" to ManageOp.LayoutExport("01JNODE/w3:t1"),
        "layout.apply" to ManageOp.LayoutApply(
            at = "01JNODE/w3:t1",
            layout = Wire.json.parseToJsonElement(
                """{"root":{"type":"split","direction":"right","ratio":0.5,
                   "children":[{"type":"pane"},{"type":"pane"}]}}"""
            ) as JsonObject,
        ),
        "session.create" to ManageOp.SessionCreate("01JNODE", "agents"),
        "session.stop" to ManageOp.SessionStop("01JNODE", "agents"),
    )

    private fun encoded(op: ManageOp) =
        Wire.json.parseToJsonElement(Wire.encode(ClientMsg.Manage(op))) as JsonObject

    @Test
    fun everyOpEncodesToTheJsonTheNodeAccepts() {
        assertEquals(cases.keys, ops.keys, "the fixture and the client must cover the same ops")
        for ((name, op) in ops) {
            assertEquals(cases.getValue(name), encoded(op), "$name")
        }
    }

    // The defect this whole seam had: `Map<String, String?>` cannot hold a float, an object, an
    // array or a tree, so four of the ops could not be sent at all.
    @Test
    fun theFourNonStringFieldsAreRealJsonTypes() {
        assertEquals("0.35", encoded(ops.getValue("pane.split"))["ratio"].toString())
        assertEquals(
            """{"RUST_LOG":"debug","KAMPR_ENV":"probe"}""",
            encoded(ops.getValue("workspace.create"))["env"].toString(),
        )
        assertEquals("""["--model","opus"]""", encoded(ops.getValue("agent.start"))["args"].toString())
        assertEquals(
            "right",
            ((encoded(ops.getValue("layout.apply"))["layout"] as JsonObject)["root"] as JsonObject)["direction"]
                .toString().trim('"'),
        )
    }

    // Clearing a pane's label is a null on the wire, not an absent key — the node reads both the
    // same way, but only one of them says what was meant.
    @Test
    fun aClearedLabelIsAnExplicitNull() {
        assertEquals(
            """{"t":"manage","op":"rename","at":"01JNODE/w3:p2","label":null}""",
            Wire.encode(ClientMsg.Manage(ManageOp.Rename("01JNODE/w3:p2", null))),
        )
    }

    @Test
    fun anEmptyEnvIsOmittedRatherThanSentAsAnEmptyObject() {
        assertEquals(
            buildJsonObject {
                put("t", "manage"); put("op", "workspace.create"); put("node", "01JNODE")
            },
            encoded(ManageOp.WorkspaceCreate(node = "01JNODE")),
        )
    }
}
