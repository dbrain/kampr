package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.Answering
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.Mark
import dev.kampr.shared.ui.MarkShape
import dev.kampr.shared.ui.PrimaryAction
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.wire.PendingOption
import dev.kampr.shared.wire.ServerMsg

// How much of the screen the card may take before its options scroll inside it. A question with
// five described options is taller than a phone, and the card is drawn *over* the transcript — so
// without a ceiling it covers the conversation it is asking about, and the reply box with it.
private val OPTIONS_CEILING = 320.dp

// What one press does. On a question that takes a single answer it *is* the answer and the dialog
// closes; on one that takes several it ticks a box and the dialog stays up until something commits
// it (#421). Naming that here rather than at each call site is what stops the two reading alike.
private fun pressLabel(option: PendingOption, multi: Boolean): String = when {
    !multi || option.text -> "Answer ${option.key}, ${option.label}"
    option.chosen -> "Untick ${option.label}"
    else -> "Tick ${option.label}"
}

// `pending.source` records whether the node lifted the question from the transcript, from the
// screen or from Claude's own hook. It is a provenance note, never a branch: the strip and the
// answer are identical either way — the footnote's words are the one thing it changes — and the
// node decides what an answer becomes. The one exception is `terminalOnly`, a hook question the
// node offers nothing to press on.
@Composable
fun PendingStrip(
    pending: ServerMsg.Pending,
    answering: Answering,
    onAnswer: (String) -> Unit,
    modifier: Modifier = Modifier,
    onSubmit: (() -> Unit)? = null,
    byKeys: Boolean = true,
    onWords: ((key: String, words: String) -> Unit)? = null,
) {
    val tokens = Kampr.tokens
    val question = pending.question ?: return
    var typing by remember(pending.id, question) { mutableStateOf<String?>(null) }
    val shape = RoundedCornerShape(tokens.radii.md)
    Column(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 9.dp)
            .background(tokens.color.blockedBg, shape)
            .edge(BorderSpec(1.dp, tokens.color.blocked), shape)
            .announce(spoken(pending), urgent = true)
            .padding(horizontal = 13.dp, vertical = 11.dp),
        verticalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Mark(tokens.color.blocked, MarkShape.Square, 7.dp)
            Column(Modifier.weight(1f)) {
                // The dialog's own title, where it draws one. Two words that say what the question
                // is *about*, which is the first thing a reader wants and the last thing a
                // truncated question sentence gives them.
                pending.header?.let { LabelText(it, tokens.type.metaSmall, tokens.color.blocked) }
                KText(question, tokens.type.bodyStrong, tokens.color.text, maxLines = 4)
            }
        }

        pending.context?.let { PendingContext(pending, it) }

        if (pending.terminalOnly) {
            KText("Answer this at the terminal.", tokens.type.micro, tokens.color.blocked, maxLines = 2)
            return@Column
        }

        // A column, not a row of chips. **The descriptions are the point** — the operator's report
        // was that the options arrived "with no context around them and the context is the most
        // important part" — and a paragraph cannot be laid beside three others on a phone. The
        // options scroll inside the card rather than growing it past the transcript.
        Column(
            Modifier
                .fillMaxWidth()
                .heightIn(max = OPTIONS_CEILING)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(7.dp),
        ) {
            pending.options.forEachIndexed { index, option ->
                // The first option is the recommended one on every dialog measured, and it is
                // never the highlighted one on a multiple-answer question — there the highlight is
                // "already ticked", which is a fact rather than a suggestion.
                val lit = if (pending.multi) option.chosen else index == 0
                val chip = RoundedCornerShape(tokens.radii.sm)
                Column(
                    Modifier
                        .fillMaxWidth()
                        .background(if (lit && answering.enabled) tokens.color.accent else tokens.color.raise, chip)
                        .edge(
                            if (lit && answering.enabled) BorderSpec(0.dp, tokens.color.accent) else tokens.card,
                            chip,
                        )
                        .touchable()
                        .action(
                            pressLabel(option, pending.multi),
                            {
                                when {
                                    !option.text || onWords == null -> onAnswer(option.key)
                                    typing == option.key -> typing = null
                                    else -> typing = option.key
                                }
                            },
                            chip,
                            enabled = answering.enabled,
                        )
                        .padding(horizontal = 12.dp, vertical = 9.dp),
                    verticalArrangement = Arrangement.spacedBy(3.dp),
                ) {
                    val ink = when {
                        !answering.enabled -> tokens.color.mute
                        lit -> tokens.color.onAccent
                        else -> tokens.color.text
                    }
                    Row(
                        Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        // The tick is drawn rather than left in the label, because the node strips
                        // it out of the label to publish `chosen` and a reader still has to see it.
                        if (pending.multi && !option.text) {
                            KText(if (option.chosen) "☑" else "☐", tokens.type.buttonSmall, ink)
                        }
                        KText("${option.key} · ${option.label}", tokens.type.buttonSmall, ink, Modifier.weight(1f))
                    }
                    option.detail?.let {
                        KText(
                            it,
                            tokens.type.micro,
                            if (lit && answering.enabled) tokens.color.onAccent else tokens.color.dim,
                            maxLines = 4,
                        )
                    }
                }
            }
        }

        // Outside the options' scroll, which is capped: inside it the field opened below the fold
        // of five described options with its Send out of sight.
        val wordsFor = pending.options.firstOrNull { it.key == typing }
        if (wordsFor != null && onWords != null) {
            PendingWords(
                wordsFor,
                enabled = answering.enabled,
                onSend = { words ->
                    onWords(wordsFor.key, words)
                    typing = null
                },
                onCancel = { typing = null },
            )
        }

        // Only on a question that takes several answers, and only where the node knows what
        // commits one on this harness. Without it the operator ticks boxes into a dialog that
        // never closes, which is worse than no affordance at all.
        if (pending.multi && onSubmit != null) {
            val ticked = pending.options.count { it.chosen }
            PrimaryAction(
                if (ticked == 0) "Tick what you want" else "Send $ticked answer${if (ticked == 1) "" else "s"}",
                onSubmit,
                Modifier.fillMaxWidth(),
                tokens.type.buttonSmall,
                9.dp,
                enabled = answering.enabled && ticked > 0,
                label = if (ticked == 1) "Send 1 answer" else "Send $ticked answers",
            )
        }

        KText(
            answering.note ?: note(pending, byKeys),
            tokens.type.micro,
            if (answering.note == null) tokens.color.mute else tokens.color.blocked,
            maxLines = 2,
        )
    }
}

// The one line under the card. On a multiple-answer question it has to say what a press does,
// because it does not do what a press on the other kind does.
private fun note(pending: ServerMsg.Pending, byKeys: Boolean): String = when {
    pending.multi -> "Takes several answers — tick what you want, then send."
    !byKeys -> "Asked by muffit, and answered to it directly."
    pending.source == "hook" -> "Asked by Claude through Kampr's hooks, and answered to it directly."
    pending.source == "transcript" -> "Read from the transcript, answered by keys into the pane."
    else -> "Read from the pane, answered by keys into the pane."
}

private fun spoken(pending: ServerMsg.Pending): String {
    val title = pending.header?.let { "$it. " }.orEmpty()
    val kind = if (pending.multi) ". Takes several answers" else ""
    val options = pending.options.joinToString(", ") { option ->
        val ticked = if (pending.multi && option.chosen) ", ticked" else ""
        listOfNotNull("${option.key} ${option.label}", option.detail).joinToString(". ") + ticked
    }
    val about = pending.context?.let { "About: $it. " }.orEmpty()
    if (pending.terminalOnly) return "${title}The agent is asking: ${pending.question}. ${about}Answer this at the terminal."
    return "${title}The agent is asking: ${pending.question}$kind. $about" +
        "${pending.options.size} answers: $options"
}
