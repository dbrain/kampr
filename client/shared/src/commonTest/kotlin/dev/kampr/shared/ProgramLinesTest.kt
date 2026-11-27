package dev.kampr.shared

import dev.kampr.shared.model.ProgramLine
import dev.kampr.shared.model.programLines
import dev.kampr.shared.wire.ProgramRow
import kotlin.test.Test
import kotlin.test.assertEquals

// docs/24: a pane's OSC 7501 records, one line each, under their parents. The node sends them most
// urgent first; a list indented by depth only reads as a tree in the tree's own order.
class ProgramLinesTest {
    @Test
    fun eachRecordIsALineIndentedByItsDepthUnderItsParent() {
        val rows = listOf(
            ProgramRow("lint/slow", "blocked", kind = "permission", title = "Slow rules", app = "ci", msg = "approve"),
            ProgramRow("test", "working", progress = 40, app = "ci"),
            ProgramRow("", "working", app = "ci"),
            ProgramRow("lint", "done", title = "Lint", app = "ci"),
            ProgramRow("a1b2", "working"),
        )
        assertEquals(
            listOf(
                ProgramLine(0, "ci", "working", null, null),
                ProgramLine(1, "a1b2", "working", null, null),
                ProgramLine(1, "ci · Lint", "done", null, null),
                ProgramLine(2, "ci · Slow rules", "blocked", null, "approve"),
                ProgramLine(1, "ci", "working", 40, null),
            ),
            programLines(rows),
        )
        assertEquals("ci, working, 40%", programLines(rows).last().spoken)
        assertEquals("ci · Slow rules, blocked, approve", programLines(rows)[3].spoken)
    }
}
