package dev.kampr.shared.model

import androidx.compose.runtime.Immutable
import dev.kampr.shared.wire.ProgramRow

// One OSC 7501 record as the pane's status lists it (docs/24): `depth` 0 is the root, and each
// `/` in an id is one level below it.
@Immutable
data class ProgramLine(val depth: Int, val name: String, val state: String, val progress: Int?, val msg: String?) {
    val spoken: String get() = listOfNotNull(name, state, progress?.let { "$it%" }, msg).joinToString(", ")
}

fun programLines(rows: List<ProgramRow>): List<ProgramLine> = rows
    .sortedWith(compareBy<ProgramRow> { it.id.isNotEmpty() }.thenBy { it.id.split('/').joinToString("\u0000") })
    .map { row ->
        ProgramLine(
            depth = if (row.id.isEmpty()) 0 else row.id.count { it == '/' } + 1,
            name = listOfNotNull(row.app, row.title).distinct().joinToString(" · ").ifEmpty { row.id.substringAfterLast('/').ifEmpty { "program" } },
            state = row.state,
            progress = row.progress,
            msg = row.msg,
        )
    }
