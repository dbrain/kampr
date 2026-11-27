package dev.kampr.shared

import dev.kampr.shared.wire.ProgramRow
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull

class ProgramWireTest {
    @Test
    fun aPaneCarriesItsProgramRecordsAndAnErrorTheStatusCannotSay() {
        val frame = """{"t":"herd","nodes":[{"id":"01JNODE"}],"panes":[
            {"id":"01JNODE/n:01ja","node_id":"01JNODE","agent_status":"idle","program_error":true,"program_msg":"boom",
             "program":[{"id":"test","state":"working","progress":40,"app":"ci"},
                        {"id":"lint/slow","state":"blocked","kind":"permission","title":"Lint","msg":"approve"}]},
            {"id":"01JNODE/n:01jb","node_id":"01JNODE"}
        ]}"""
        val (failed, plain) = (Wire.decode(frame) as ServerMsg.Herd).panes
        assertEquals(true, failed.programError)
        assertEquals("boom", failed.programMsg)
        assertEquals(
            listOf(
                ProgramRow("test", "working", progress = 40, app = "ci"),
                ProgramRow("lint/slow", "blocked", kind = "permission", title = "Lint", msg = "approve"),
            ),
            failed.program,
        )
        assertFalse(plain.programError)
        assertNull(plain.programMsg)
        assertNull(plain.program)
    }
}
