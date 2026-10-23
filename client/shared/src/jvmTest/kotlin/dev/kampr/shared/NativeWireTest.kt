package dev.kampr.shared

import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NativeExit
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals

// Inline rather than in `manage-ops.json`: that fixture belongs to both sides and `ManageWireTest`
// requires the two key sets to match, so the native cases move there once the node reads them too.
class NativeWireTest {
    @Test
    fun everyNativeOpEncodesToTheContractShape() {
        val at = "01JNODE/n:01jsession"
        val cases = listOf(
            ManageOp.NativeCreate("01JNODE") to """{"t":"manage","op":"native.create","node":"01JNODE"}""",
            ManageOp.NativeCreate(
                "01JNODE",
                command = "cargo test && make",
                cwd = "/home/dbrain/dev/kampr",
                label = "build",
            ) to """{"t":"manage","op":"native.create","node":"01JNODE","command":"cargo test && make","cwd":"/home/dbrain/dev/kampr","label":"build"}""",
            ManageOp.NativeCreate("01JNODE", args = listOf("claude", "--model", "opus")) to
                """{"t":"manage","op":"native.create","node":"01JNODE","args":["claude","--model","opus"]}""",
            ManageOp.NativeCreate("01JNODE", desk = true, at = "01JNODE/w3:p2") to
                """{"t":"manage","op":"native.create","node":"01JNODE","desk":true,"at":"01JNODE/w3:p2"}""",
            ManageOp.NativeCreate("01JNODE", desk = true) to """{"t":"manage","op":"native.create","node":"01JNODE","desk":true}""",
            // An `at` is routed; without `desk` it would only send the op chasing a herdr pane.
            ManageOp.NativeCreate("01JNODE", at = "01JNODE/w3:p2") to """{"t":"manage","op":"native.create","node":"01JNODE"}""",
            ManageOp.NativeClose(at) to """{"t":"manage","op":"native.close","at":"$at"}""",
            ManageOp.NativeForget(at) to """{"t":"manage","op":"native.forget","at":"$at"}""",
            ManageOp.NativeDesk(at) to """{"t":"manage","op":"native.desk","at":"$at"}""",
        )
        for ((op, json) in cases) {
            assertEquals(json, Wire.encode(ClientMsg.Manage(op)), "$op")
        }
    }

    @Test
    fun aPaneRecordCarriesTheNativeFactsAndAHerdrOneDefaultsToNone() {
        val frame = """{"t":"herd","nodes":[{"id":"01JNODE"}],"panes":[
            {"id":"01JNODE/w1:p1","node_id":"01JNODE"},
            {"id":"01JNODE/n:01ja","node_id":"01JNODE","provider":"native","session_id":"01ja",
             "on_desk":"w1:p3","detached":false,"size_holder":"attach:4242"},
            {"id":"01JNODE/n:01jb","node_id":"01JNODE","provider":"native","session_id":"01jb",
             "detached":true,"exited":{"signal":9}},
            {"id":"01JNODE/n:01jc","node_id":"01JNODE","provider":"native","session_id":"01jc",
             "detached":true,"exited":{"code":0}}
        ]}"""
        val panes = (Wire.decode(frame) as ServerMsg.Herd).panes
        assertEquals(
            listOf(
                PaneInfo("01JNODE/w1:p1", "01JNODE"),
                PaneInfo(
                    "01JNODE/n:01ja",
                    "01JNODE",
                    provider = "native",
                    sessionId = "01ja",
                    onDesk = "w1:p3",
                    sizeHolder = "attach:4242",
                ),
                PaneInfo("01JNODE/n:01jb", "01JNODE", provider = "native", sessionId = "01jb", detached = true, exited = NativeExit(signal = 9)),
                PaneInfo("01JNODE/n:01jc", "01JNODE", provider = "native", sessionId = "01jc", detached = true, exited = NativeExit(code = 0)),
            ),
            panes,
        )
    }
}
