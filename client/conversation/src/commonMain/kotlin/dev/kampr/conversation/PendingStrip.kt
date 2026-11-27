package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.Measurable
import androidx.compose.ui.layout.Placeable
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
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
import kotlinx.serialization.json.JsonObject

// How much of the screen the card may take before its options scroll inside it. A question with
// five described options is taller than a phone, and the card is drawn *over* the transcript — so
// without a ceiling it covers the conversation it is asking about, and the reply box with it.
private val OPTIONS_CEILING = 320.dp

// A form taller than this scrolls inside the card, which is itself capped by the pane.
private val FORM_CEILING = 420.dp

// What the question's context (a command, a plan) keeps while the answers take their room: it can
// be scrolled, and a sliver of it says there is something to scroll.
private val CONTEXT_FLOOR = 72.dp

private val GAP = 9.dp

// Side by side wherever both halves are still wide enough to read.
private val SIDE_BY_SIDE_WIDTH = 560.dp

// Finite, because the answers weigh their options against the words field and a weight in an
// unbounded column is measured at nothing.
private const val UNBOUNDED = 1 shl 20

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
    onForm: ((JsonObject) -> Unit)? = null,
    agent: String? = null,
    // The height the pane can spare for the card. The card is drawn over the transcript, and on a
    // rotated phone an uncapped one pushed its own answers out of the window.
    maxHeight: Dp = Dp.Unspecified,
) {
    val tokens = Kampr.tokens
    val question = pending.question ?: return
    var typing by remember(pending.id, question) { mutableStateOf<String?>(null) }
    var filling by remember(pending.id, question) { mutableStateOf(false) }
    val shape = RoundedCornerShape(tokens.radii.md)
    CardStack(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 9.dp)
            .heightIn(max = maxHeight)
            .background(tokens.color.blockedBg, shape)
            .edge(BorderSpec(1.dp, tokens.color.blocked), shape)
            .announce(spoken(pending), urgent = true)
            .padding(horizontal = 13.dp, vertical = 11.dp),
        head = {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Mark(tokens.color.blocked, MarkShape.Square, 7.dp)
                Column(Modifier.weight(1f)) {
                    // The dialog's own title, where it draws one. Two words that say what the
                    // question is *about*, which is the first thing a reader wants and the last
                    // thing a truncated question sentence gives them.
                    pending.header?.let { LabelText(it, tokens.type.metaSmall, tokens.color.blocked) }
                    KText(question, tokens.type.bodyStrong, tokens.color.text, maxLines = 4)
                }
            }
        },
        context = {
            // A form's fields described one per line are what the form itself draws, so the
            // description goes once the form is open.
            val described = if (filling && pending.context == null) null else pending.described
            described?.let { PendingContext(pending, it) }
        },
        answers = {
            if (pending.terminalOnly) {
                KText("Answer this at the terminal.", tokens.type.micro, tokens.color.blocked, maxLines = 2)
                return@CardStack
            }
            Column(verticalArrangement = Arrangement.spacedBy(GAP)) {
                // A column, not a row of chips. **The descriptions are the point** — the
                // operator's report was that the options arrived "with no context around them and
                // the context is the most important part" — and a paragraph cannot be laid beside
                // three others on a phone. The options scroll inside the card rather than growing
                // it past the transcript.
                // An open form is answered by its own Send and put away by its Cancel, so the
                // answers it was opened from give their room to its fields.
                if (!filling) {
                    val optionsScroll = rememberScrollState()
                    Box(
                        Modifier
                            .fillMaxWidth()
                            // Weighed against an open form only by its own height: two weights share
                            // what is left evenly, and two short options took half the form's room.
                            .then(if (filling) Modifier else Modifier.weight(1f, fill = false))
                            .heightIn(max = OPTIONS_CEILING),
                    ) {
                        Column(
                            Modifier.fillMaxWidth().verticalScroll(optionsScroll),
                            verticalArrangement = Arrangement.spacedBy(7.dp),
                        ) {
                            pending.options.forEachIndexed { index, option ->
                                // The first option is the recommended one on every dialog measured, and it
                                // is never the highlighted one on a multiple-answer question — there the
                                // highlight is "already ticked", which is a fact rather than a suggestion.
                                val lit = if (pending.multi) option.chosen else index == 0
                                AnswerChip(
                                    pressLabel(option, pending.multi),
                                    lit,
                                    answering.enabled,
                                    {
                                        when {
                                            pending.form && option.key == "1" && onForm != null -> filling = !filling
                                            !option.text || onWords == null -> onAnswer(option.key)
                                            typing == option.key -> typing = null
                                            else -> typing = option.key
                                        }
                                    },
                                    Modifier.fillMaxWidth(),
                                ) { ink ->
                                    Row(
                                        Modifier.fillMaxWidth(),
                                        verticalAlignment = Alignment.CenterVertically,
                                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                                    ) {
                                        // The tick is drawn rather than left in the label, because the node
                                        // strips it out of the label to publish `chosen` and a reader still
                                        // has to see it.
                                        if (pending.multi && !option.text) {
                                            KText(if (option.chosen) "☑" else "☐", tokens.type.buttonSmall, ink)
                                        }
                                        // Three lines: "don't ask again" options carry their scope at the
                                        // end of the label, and the scope is the part that matters.
                                        KText("${option.key} · ${option.label}", tokens.type.buttonSmall, ink, Modifier.weight(1f), maxLines = 3)
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
                        MoreBelow(optionsScroll, tokens.color.blockedBg, Modifier.align(Alignment.BottomStart))
                    }
                }

                // Outside the options' scroll, which is capped: inside it the field opened below
                // the fold of five described options with its Send out of sight.
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

                if (filling && onForm != null) {
                    PendingForm(
                        pending.fields,
                        enabled = answering.enabled,
                        onSend = { values ->
                            onForm(values)
                            filling = false
                        },
                        onCancel = { filling = false },
                        Modifier.weight(1f, fill = false).heightIn(max = FORM_CEILING),
                    )
                }
            }
        },
        foot = {
            if (pending.terminalOnly) return@CardStack
            Column(verticalArrangement = Arrangement.spacedBy(GAP)) {
                // Only on a question that takes several answers, and only where the node knows
                // what commits one on this harness. Without it the operator ticks boxes into a
                // dialog that never closes, which is worse than no affordance at all.
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
                    answering.note ?: note(pending, byKeys, agent),
                    tokens.type.micro,
                    if (answering.note == null) tokens.color.mute else tokens.color.blocked,
                    maxLines = 2,
                )
            }
        },
    )
}

// The card's four parts, measured by what must survive a short window rather than in reading
// order: the question, then the foot (a multiple-answer question's Send lives there), then the
// answers, and the context last — it scrolls in its own box, keeps at least a sliver, and is the
// part a reader can scroll to. Placed in reading order. A plain column measured them top-down,
// so on a rotated phone a long plan took the whole card and its answers were cut off below it.
//
// A card wider than a phone has width to spare and not height, so there the context and the
// answers stand side by side, each with the whole body's height: stacked, the answers took the
// body and a plan on a rotated phone or a desk was two lines tall.
@Composable
private fun CardStack(
    modifier: Modifier,
    head: @Composable () -> Unit,
    context: @Composable () -> Unit,
    answers: @Composable () -> Unit,
    foot: @Composable () -> Unit,
) {
    Layout(contents = listOf(head, context, answers, foot), modifier = modifier) { (headM, contextM, answersM, footM), constraints ->
        val width = constraints.maxWidth
        val bounded = constraints.hasBoundedHeight
        val gap = GAP.roundToPx()
        fun slot(measurables: List<Measurable>, height: Int, wide: Int = width) = measurables.map {
            it.measure(Constraints(minWidth = wide, maxWidth = wide, maxHeight = if (bounded) height.coerceAtLeast(0) else UNBOUNDED))
        }
        fun tall(placeables: List<Placeable>) = placeables.sumOf { it.height }
        var left = constraints.maxHeight
        val headP = slot(headM, left)
        left -= tall(headP)
        val footP = slot(footM, left - gap)
        if (tall(footP) > 0) left -= tall(footP) + gap
        val body = left - gap
        val beside = bounded && contextM.isNotEmpty() && width >= SIDE_BY_SIDE_WIDTH.roundToPx()
        if (beside) {
            val half = (width - gap) / 2
            val answersP = slot(answersM, body, half)
            val contextP = slot(contextM, body, width - gap - half)
            val middle = maxOf(tall(answersP), tall(contextP))
            val height = tall(headP) + gap + middle + if (tall(footP) > 0) gap + tall(footP) else 0
            return@Layout layout(width, height.coerceIn(constraints.minHeight, constraints.maxHeight)) {
                val top = tall(headP) + gap
                fun List<Placeable>.stack(x: Int, from: Int) = fold(from) { y, it ->
                    it.place(x, y)
                    y + it.height
                }
                headP.stack(0, 0)
                contextP.stack(0, top)
                answersP.stack(width - half, top)
                footP.stack(0, top + middle + gap)
            }
        }
        val share = if (contextM.isEmpty() || !bounded) 0 else minOf(CONTEXT_FLOOR.roundToPx(), left / 3) + gap
        val answersP = slot(answersM, left - gap - share)
        if (tall(answersP) > 0) left -= tall(answersP) + gap
        val contextP = slot(contextM, left - gap)
        val parts = listOf(headP, contextP, answersP, footP).filter { tall(it) > 0 }
        val height = parts.sumOf { tall(it) } + gap * (parts.size - 1).coerceAtLeast(0)
        layout(width, height.coerceIn(constraints.minHeight, if (bounded) constraints.maxHeight else Int.MAX_VALUE)) {
            var y = 0
            parts.forEach { part ->
                part.forEach {
                    it.place(0, y)
                    y += it.height
                }
                y += gap
            }
        }
    }
}

// The one line under the card. On a multiple-answer question it has to say what a press does,
// because it does not do what a press on the other kind does.
private fun note(pending: ServerMsg.Pending, byKeys: Boolean, agent: String?): String = when {
    pending.multi -> "Takes several answers — tick what you want, then send."

    // A node before 0.1.154 marks a conversation source's question as read from the transcript,
    // and a pane with no terminal is the one place that cannot be true.
    pending.source == "conversation" || !byKeys -> "Asked by ${agent ?: "the agent"}, and answered to it directly."

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
