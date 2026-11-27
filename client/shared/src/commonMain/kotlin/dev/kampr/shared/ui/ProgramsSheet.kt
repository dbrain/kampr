package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.ProgramLine
import dev.kampr.shared.model.programLines
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.ProgramRow

// docs/24: every record a pane's programs reported, under its parent, with the herd's marks for
// their states. An `error` wears the warning the header gives a failed program.
@Composable
fun ProgramsSheet(paneName: String, rows: List<ProgramRow>, breakpoint: Breakpoint, onDismiss: () -> Unit) {
    val compact = breakpoint != Breakpoint.Portrait
    BottomSheet(breakpoint, onDismiss) {
        SheetHeader("Programs", paneName, null, onDismiss, compact = compact)
        Column(
            Modifier
                .weight(1f, fill = false)
                .verticalScroll(rememberScrollState())
                .padding(start = 20.dp, end = 20.dp, bottom = 20.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            for (line in programLines(rows)) ProgramLineRow(line)
        }
    }
}

private fun stateOf(word: String): AgentStatus? = when (word) {
    "blocked" -> AgentStatus.Blocked
    "working" -> AgentStatus.Working
    "done" -> AgentStatus.Done
    "idle" -> AgentStatus.Idle
    else -> null
}

@Composable
private fun ProgramLineRow(line: ProgramLine) {
    val tokens = Kampr.tokens
    Row(
        Modifier
            .fillMaxWidth()
            .padding(start = (line.depth * 14).dp)
            .clearAndSetSemantics { contentDescription = line.spoken },
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Box(Modifier.padding(top = 4.dp).size(10.dp), contentAlignment = Alignment.Center) {
            when (val status = stateOf(line.state)) {
                null -> IconGlyph(KamprIcons.warning, 10.dp, tokens.color.blocked)
                else -> StatusMark(status, 7.dp)
            }
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            KText(line.name, tokens.type.cardTitle, tokens.color.text, overflow = ELIDE_MIDDLE)
            KText(listOfNotNull(line.state, line.progress?.let { "$it%" }).joinToString(" · "), tokens.type.micro, tokens.color.dim)
            line.msg?.let { KText(it, tokens.type.captionSmall, tokens.color.dim, maxLines = 3) }
        }
    }
}
