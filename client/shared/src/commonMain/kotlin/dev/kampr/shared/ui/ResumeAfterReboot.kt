package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.homeRelative
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.util.displayArgv
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.ResumeRecord

private fun programOf(record: ResumeRecord): String = record.argv.firstOrNull()?.substringAfterLast('/')?.ifEmpty { null } ?: "command"

private fun resumeSpoken(record: ResumeRecord): String = listOfNotNull(
    programOf(record),
    record.cwd?.let { "in ${homeRelative(it)}" },
    "runs ${displayArgv(record.argv)}",
    record.label?.let { "was $it" },
).joinToString(", ")

// docs/25, under `[native] resume = "ask"`: what a boot left on this machine, each record with
// what would run spelled out, since the tap is the only consent it gets. Nothing at all where the
// machine has none, keeps no sessions, or predates the op.
@Composable
fun ResumeAfterReboot(node: NodeInfo, look: HerdLook, modifier: Modifier = Modifier) {
    val manage = LocalManage.current
    if (!manage.enabled) return
    val records = manage.resumable(node.id)
    if (records.isEmpty()) return
    val tokens = Kampr.tokens
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(look.gap)) {
        LabelText(
            "Resume after reboot",
            tokens.type.captionSmall,
            tokens.color.mute,
            Modifier
                .padding(start = look.inset, top = look.headingTop, bottom = look.headingBottom)
                .asHeading()
                .named("Resume after reboot on ${node.name}"),
        )
        for (record in records) ResumeRow(node, record, look)
    }
}

@Composable
private fun ResumeRow(node: NodeInfo, record: ResumeRecord, look: HerdLook) {
    val tokens = Kampr.tokens
    val manage = LocalManage.current
    val shape = RoundedCornerShape(tokens.radii.lg)
    val program = programOf(record)
    val spoken = resumeSpoken(record)
    Column(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.raise, shape)
            .edge(tokens.card, shape)
            .padding(horizontal = 14.dp, vertical = 11.dp),
        verticalArrangement = Arrangement.spacedBy(if (look.rows) 6.dp else 8.dp),
    ) {
        Column(Modifier.clearAndSetSemantics { contentDescription = spoken }, verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                KText(program, tokens.type.cardTitle, tokens.color.text, Modifier.weight(1f, fill = false))
                record.label?.let { KText(it, tokens.type.micro, tokens.color.accent) }
            }
            record.cwd?.let { KText(homeRelative(it), tokens.type.meta, tokens.color.mute, overflow = ELIDE_MIDDLE) }
            KText(displayArgv(record.argv), tokens.type.meta, tokens.color.dim, maxLines = 3)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(9.dp)) {
            PrimaryAction(
                "Resume",
                { manage.resume(node.id, record) },
                Modifier.weight(1f),
                tokens.type.buttonSmall,
                10.dp,
                label = "Resume $program",
            )
            QuietAction(
                "Dismiss",
                { manage.dismissResume(node.id, record) },
                Modifier.weight(1f),
                label = "Dismiss $program",
            )
        }
    }
}
