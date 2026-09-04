package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.balanced
import dev.kampr.shared.model.secretish
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.wire.FleetCommand
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.ServerMsg

// What the operator is about to run everywhere, and what the node remembers they ran before.
//
// **The line is shown back before it fans out.** There used to be a note here explaining that
// there was no shell and that `;` was an argument — an instruction to work around a limitation,
// which the limitation no longer has. What replaced it is not nothing: a preview of the exact line
// that is about to run, beside the number of machines it is about to run on. Those are the two
// facts somebody pressing this can be surprised by, and neither of them is a rule to remember.
//
// **Pressing a remembered command stages it; it never runs it.** A saved command is a one-press
// fan-out across every machine in the herd, and it fires through exactly the confirmation a typed
// one does — the same host count on the same button, with the line in the box where it can be read
// and edited first. One press across the whole herd should not be cheaper than typing it out.
//
// The host count is on the button because the number of machines is the part of this decision that
// is easy to be wrong about, and it is resolved *now* rather than remembered: a saved entry carries
// no host selection, so it always means "everywhere I can reach today".
@Composable
internal fun RunSheet(
    hosts: Int,
    book: ServerMsg.FleetBook,
    // What the box opens holding, when something outside the sheet chose it — the empty board's
    // quick links. It is staged and not run: the button below is still the only thing that fires.
    staged: FleetCommand? = null,
    breakpoint: Breakpoint,
    onCancel: () -> Unit,
    onRun: (String) -> Unit,
    onBook: (ManageOp) -> Unit,
) {
    val tokens = Kampr.tokens
    var entry by remember {
        mutableStateOf(
            staged?.command.orEmpty().let { TextFieldValue(it, TextRange(it.length)) },
        )
    }
    var naming by remember { mutableStateOf(TextFieldValue(staged?.label.orEmpty())) }
    val line = entry.text.trim()
    val closed = balanced(entry.text)
    val ready = closed && line.isNotEmpty()
    // One argument, because that is what the line is now: the node hands it to a shell whole, and
    // the rule that reads it flattens on whitespace exactly as it always did for `sh -c '…'`.
    val carries = if (ready) secretish(listOf(line)) else null

    fun stage(command: FleetCommand) {
        val text = command.command
        entry = TextFieldValue(text, TextRange(text.length))
        naming = TextFieldValue(command.label.orEmpty())
    }

    BottomSheet(breakpoint, onCancel) {
        Column(
            modifier = Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            KText("Run on $hosts machine${if (hosts == 1) "" else "s"}", tokens.type.paneTitle, tokens.color.text)
            KField(
                hint = "pacman -Syu",
                value = entry,
                onSubmit = { if (ready) onRun(line) },
                onChange = { entry = it },
            )
            // The preview, and it is the command itself rather than a description of it. A person
            // pressing Run on five machines should be able to read the line off this screen — the
            // box above it is where they were typing and is easy to have scrolled.
            if (ready) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .background(tokens.color.surface2, RoundedCornerShape(tokens.radii.md))
                        .padding(horizontal = 12.dp, vertical = 9.dp),
                    verticalArrangement = Arrangement.spacedBy(3.dp),
                ) {
                    KText(
                        "Will run on $hosts machine${if (hosts == 1) "" else "s"}",
                        tokens.type.caption,
                        tokens.color.mute,
                    )
                    KText(line, tokens.type.body, tokens.color.accent, maxLines = 4)
                }
            }
            KText(
                // The shell is the answer to `&&` and `|`, and the two things it does *not* bring
                // are worth one clause here rather than a surprise on five machines at once.
                if (!closed) {
                    "That command has a quote that never closes."
                } else {
                    "Runs through each machine's own shell, so `&&`, `|`, `;`, quotes and globs all work. " +
                        "Aliases and shell functions do not — those live in .bashrc, which a run does not read."
                },
                tokens.type.captionSmall,
                if (!closed) tokens.color.blocked else tokens.color.mute,
                maxLines = 4,
            )
            if (ready) {
                KField(
                    hint = "Name it (optional)",
                    value = naming,
                    onChange = { naming = it },
                )
            }
            // The node declines to write a secret-shaped command down by itself and allows a
            // deliberate save; this is where the operator finds out which of the two is happening.
            // It is a reduction, not a filter — `./deploy hunter2` says nothing here — so the
            // wording promises a look rather than a guarantee.
            carries?.let {
                KText(
                    "This looks like it carries $it. It will not be remembered on its own; saving it " +
                        "writes it to the node's disk and shows it on every device.",
                    tokens.type.captionSmall,
                    tokens.color.blocked,
                    maxLines = 4,
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                FleetChip("Cancel", isDefault = false, onClick = onCancel)
                if (ready) {
                    FleetChip("Save", isDefault = false) {
                        // One element, and that element is the whole line: an entry is rendered by
                        // joining `args` with spaces, here and on every older client still on a
                        // phone, so this reads back byte for byte as it was typed.
                        onBook(ManageOp.FleetSave(args = listOf(line), label = naming.text.trim().ifEmpty { null }))
                    }
                    FleetChip("Run everywhere", isDefault = true) { onRun(line) }
                }
            }
            BookSection(
                title = "Saved",
                empty = "Nothing kept yet. Type a command and press Save.",
                commands = book.saved,
                onStage = ::stage,
                onBook = onBook,
            )
            BookSection(
                title = "Recent",
                empty = "No commands yet. What you run here shows up in this list.",
                commands = book.recent,
                onStage = ::stage,
                onBook = onBook,
            )
        }
    }
}

@Composable
private fun BookSection(
    title: String,
    empty: String,
    commands: List<FleetCommand>,
    onStage: (FleetCommand) -> Unit,
    onBook: (ManageOp) -> Unit,
) {
    val tokens = Kampr.tokens
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        KText(title, tokens.type.caption, tokens.color.mute, Modifier.asHeading())
        // An empty list says what would fill it, so a fresh node reads as deliberate rather than
        // broken. There is nothing to fetch and nothing has gone wrong.
        if (commands.isEmpty()) {
            KText(empty, tokens.type.captionSmall, tokens.color.mute, maxLines = 2)
            return@Column
        }
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .background(tokens.color.surface2, RoundedCornerShape(tokens.radii.md)),
        ) {
            commands.forEach { command -> BookRow(command, onStage, onBook) }
        }
    }
}

@Composable
private fun BookRow(
    command: FleetCommand,
    onStage: (FleetCommand) -> Unit,
    onBook: (ManageOp) -> Unit,
) {
    val tokens = Kampr.tokens
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Column(
            modifier = Modifier
                .weight(1f)
                .action("Put ${command.command} in the box", { onStage(command) }),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            // **The label never replaces the command.** "deploy staging" reads better than the
            // argv and tells the operator nothing about what is about to run on every machine in
            // the herd, so both are on screen whenever there is a label at all.
            command.label?.let { KText(it, tokens.type.bodyStrong, tokens.color.text, maxLines = 1) }
            KText(command.command, tokens.type.captionSmall, tokens.color.accent, maxLines = 2)
        }
        if (command.label == null) {
            KText(
                "Keep",
                tokens.type.captionSmall,
                tokens.color.mute,
                Modifier.action("Keep ${command.command}", { onBook(ManageOp.FleetSave(entry = command.id)) }),
            )
        }
        // The rule that keeps credentials out of the automatic half is a reduction rather than a
        // filter, so this is the part that actually holds: anything written down can be removed.
        KText(
            "Forget",
            tokens.type.captionSmall,
            tokens.color.mute,
            Modifier.action("Forget ${command.command}", { onBook(ManageOp.FleetDrop(command.id)) }),
        )
    }
}

@Composable
internal fun FleetChip(label: String, isDefault: Boolean, enabled: Boolean = true, onClick: () -> Unit) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    Box(
        modifier = Modifier
            .background(if (isDefault && enabled) tokens.color.accent else tokens.color.raise, shape)
            .action(label, onClick, shape, enabled = enabled)
            .padding(horizontal = 16.dp, vertical = 9.dp),
    ) {
        DisableSelection {
            KText(
                label,
                tokens.type.button,
                when {
                    !enabled -> tokens.color.mute
                    isDefault -> tokens.color.onAccent
                    else -> tokens.color.text
                },
            )
        }
    }
}
