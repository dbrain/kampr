package dev.kampr.conversation

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.LIVE_TURN_ID
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.net.wallClockMillis
import dev.kampr.shared.platform.LocalReduceMotion
import dev.kampr.shared.theme.BorderSpec
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.ui.BottomSheet
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KField
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LabelText
import dev.kampr.shared.ui.Mark
import dev.kampr.shared.ui.MarkShape
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.PrimaryAction
import dev.kampr.shared.ui.SheetHeader
import dev.kampr.shared.ui.SheetSection
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.edge
import dev.kampr.shared.ui.edgeBottom
import dev.kampr.shared.ui.readingOrder
import dev.kampr.shared.ui.touchable
import dev.kampr.shared.util.elapsedSpan
import dev.kampr.shared.util.parseIsoMillis
import dev.kampr.shared.wire.Abilities
import dev.kampr.shared.wire.AbilityWords
import dev.kampr.shared.wire.Action
import dev.kampr.shared.wire.ArgSpec
import dev.kampr.shared.wire.Billing
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Control
import dev.kampr.shared.wire.Limits
import dev.kampr.shared.wire.LiveTurn
import dev.kampr.shared.wire.Reply
import dev.kampr.shared.wire.Suggestion
import dev.kampr.shared.wire.Task
import dev.kampr.shared.wire.TroubleFix
import dev.kampr.shared.wire.Turn
import dev.kampr.shared.wire.canInterrupt
import dev.kampr.shared.wire.canMention
import dev.kampr.shared.wire.canSendNow
import dev.kampr.shared.wire.canSlash
import kotlinx.coroutines.delay
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put

// The conversation seam's pane pieces (docs/21 Phase 2). Every one of them is drawn from what
// the source declared — `abilities` and the frames it sends — and every one of them takes no
// room at all when there is nothing to show: no empty rows, no disabled controls, no gaps.

// The token count a context line says, in the units a reader thinks in.
fun compactTokens(n: Long): String = when {
    n >= 1_000_000 -> "${n / 1_000_000}.${(n % 1_000_000) / 100_000}M"
    n >= 1_000 -> "${n / 1_000}k"
    else -> n.toString()
}

// What the billing line says of a session. `provider` is who the session runs on, `key` is which
// key pays, and `rateLimited` is the provider saying it is throttling: each is its own fact and
// only the ones present are said.
fun billingWord(billing: Billing): String? {
    val who = billing.provider ?: billing.key?.let { "key $it" } ?: return null
    return if (billing.rateLimited) "$who, rate-limited" else "billed by $who"
}

// The limits line, in the words the source gave. `usage` is an open value the provider counts in
// its own units, so it is said only where it is a single readable number or word — an object the
// client did not ask for is not one the client guesses at.
fun limitsWord(limits: Limits): String? {
    val parts = buildList {
        limits.plan?.let { add(it) }
        limits.usage?.let { u ->
            when {
                u is JsonPrimitive && u.isString -> add(u.content)
                u is JsonPrimitive && u.contentOrNull?.toLongOrNull() != null -> add("${u.contentOrNull}%")
                else -> Unit
            }
        }
        limits.resetsAt?.let { add("resets $it") }
    }
    return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
}

// The word a control's value is drawn as: a primitive is its content, and a structure the source
// sent is said in its own JSON rather than guessed at.
fun valueWord(value: JsonElement?): String? = when (value) {
    is JsonPrimitive -> value.contentOrNull
    null -> null
    else -> value.toString()
}

// The word a task row is called by, in the order the source says it: the title it gave, the call
// it runs, and the id only as a last resort — an id on screen is a coordinate, not a name.
fun taskLabel(task: Task): String = task.title?.trim()?.takeIf { it.isNotEmpty() }
    ?: task.call?.trim()?.takeIf { it.isNotEmpty() }
    ?: task.id

// The message being written, as a turn the transcript can draw: the text so far and the tool
// calls as their arguments arrive. It carries the live turn's id, which is what the transcript's
// own `Reply.live` already reads, and it is withdrawn with every revision rather than appended,
// for the reason the recorded live preview is.
fun liveTurnOf(live: LiveTurn?): Turn? {
    if (live == null) return null
    val blocks = buildList {
        if (live.text.isNotEmpty()) add(Block.Md(live.text))
        live.tools.forEach { add(Block.Tool(it.tool, null, null, it.state)) }
    }
    return if (blocks.isEmpty()) null else Turn(id = LIVE_TURN_ID, role = "assistant", blocks = blocks)
}

// A permission-style answer, as the seam's own `Reply`: the node's `key_of` maps the same three
// keys to the same words, so a node that has never heard of the field behaves exactly as it did,
// and one that routes the reply receives the structured answer the dock meant.
fun permissionReply(key: String): Reply? = when (key) {
    "1" -> Reply.Allow()
    "2" -> Reply.Allow(remember = true)
    "3" -> Reply.Deny()
    else -> null
}

// What the source is being asked to complete, where it completes anything at all: a line that
// starts on a `/` the source declared `slash` for, and a word after a `@` it declared `mention`
// for. A `/` in the middle of a sentence is a word, and an `@` on a source without `mention` is
// one too — so the ask goes out only where the declaration says a frame will come back.
fun suggestQuery(draft: String, abilities: Abilities): Pair<String, String>? = when {
    draft.startsWith("/") && abilities.canSlash -> "commands" to draft.removePrefix("/")

    '@' in draft && abilities.canMention -> {
        val at = draft.lastIndexOf('@')
        "files" to draft.substring(at + 1)
    }

    else -> null
}

// A trouble is one line and one button under the header, and the button is the fix: where the
// fix is to wait, there is no button to press, and a line that says so takes the row rather than
// a disabled control taking it.
@Composable
fun TroubleBanner(
    pane: PaneState,
    onOpenSession: () -> Unit,
    onTerminal: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val trouble = pane.trouble ?: return
    val tokens = Kampr.tokens
    val fix = trouble.fix
    val say = buildList {
        add(trouble.text)
        trouble.retry?.let { add("retry ${it.attempt} of ${it.of} in ${it.inMs / 1000}s") }
        if (fix is TroubleFix.Wait) add("held until ${fix.until}")
    }.joinToString(". ")
    Row(
        modifier
            .fillMaxWidth()
            .background(tokens.color.blockedBg)
            .edgeBottom(BorderSpec(1.dp, tokens.color.blocked))
            .readingOrder(-0.9f)
            .announce("Trouble: $say", urgent = true)
            .padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Mark(tokens.color.blocked, MarkShape.Square, 7.dp)
        KText(say, tokens.type.meta, tokens.color.text, Modifier.weight(1f), maxLines = 2)
        when (fix) {
            TroubleFix.Login -> FixButton("Open the terminal", onTerminal, tokens)
            TroubleFix.Billing -> FixButton("Options", onOpenSession, tokens)
            else -> Unit
        }
    }
}

@Composable
private fun FixButton(label: String, onFix: () -> Unit, tokens: KamprTokens) {
    val shape = RoundedCornerShape(tokens.radii.sm)
    KText(
        label,
        tokens.type.buttonSmall,
        tokens.color.onAccent,
        Modifier
            .background(tokens.color.blocked, shape)
            .touchable(LANDSCAPE_TOUCH)
            .action(label, onFix, shape)
            .padding(horizontal = 10.dp, vertical = 5.dp),
    )
}

// The facts line under the composer: one line that says what the header details would say at
// length, and takes no room at all where the source declares no drawn fact or the facts have
// not arrived. A tap opens the session sheet, which is the whole of the detail.
@Composable
fun FactsLine(
    pane: PaneState,
    onOpen: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val tokens = Kampr.tokens
    if (pane.abilities.facts.none { it in AbilityWords.FACTS_DRAWN }) return
    val parts = buildList {
        pane.session?.model?.let { add(it) }
        pane.context?.let { add("context ${compactTokens(it.used)}/${compactTokens(it.total)}") }
        pane.limits?.let { limitsWord(it) }?.let { add(it) }
        pane.session?.billing?.let { billingWord(it) }?.let { add(it) }
    }
    if (parts.isEmpty()) return
    Row(
        modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeBottom()
            .touchable()
            .action("Open the session details", onOpen)
            .padding(horizontal = 12.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        IconGlyph(ConversationIcons.speech, 11.dp, tokens.color.dim)
        KText(
            parts.joinToString(" · "),
            tokens.type.micro,
            tokens.color.dim,
            Modifier.weight(1f),
            maxLines = 1,
        )
        IconGlyph(ConversationIcons.chevronUp, 10.dp, tokens.color.mute)
    }
}

const val TASKS_OPEN = "tasks:open"

// What a task's row says out loud: the name, the kind, and how long it has been running.
fun taskSpoken(task: Task, nowMillis: Double): String {
    val since = parseIsoMillis(task.since)?.let { elapsedSpan(nowMillis - it) }
    val state = if (task.state == "ended") ", ended" else ""
    return listOfNotNull(taskLabel(task), task.kind.takeIf { it.isNotEmpty() }, since?.let { "running $it" })
        .joinToString(", ") + state
}

// The task panel: the running-strip's shape, with rows that open something. An agent's row opens
// the conversation it is running; a shell's row opens its output. A task with a parent is the
// subagent's own shell and sits under it, indented, for the reason a launch sits under the turn
// that started it.
@Composable
fun ActivityBar(
    pane: PaneState,
    onOpenTask: (Task) -> Unit,
    modifier: Modifier = Modifier,
    open: Boolean = false,
    onFold: () -> Unit = {},
    clock: () -> Double = ::wallClockMillis,
) {
    val tasks = pane.tasks
    if (tasks.isEmpty()) return
    val tokens = Kampr.tokens
    var now by remember { mutableStateOf(clock()) }
    LaunchedEffect(tasks.map { it.id }) {
        now = clock()
        while (true) {
            delay(1000)
            now = clock()
        }
    }
    val running = tasks.count { it.state != "ended" }
    val shape = RoundedCornerShape(tokens.radii.md)
    val foldable = tasks.size > 1
    val shown = if (foldable && !open) emptyList() else tasks
    DisableSelection {
        Column(
            modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 6.dp)
                .background(tokens.color.raise, shape)
                .edge(tokens.card, shape)
                .announce(
                    if (shown.isEmpty()) {
                        "$running running, folded: ${tasks.joinToString { taskSpoken(it, now) }}"
                    } else {
                        tasks.joinToString("; ") { taskSpoken(it, now) }
                    },
                )
                .padding(horizontal = 12.dp, vertical = 9.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .then(
                        if (!foldable) {
                            Modifier
                        } else {
                            Modifier
                                .touchable(LANDSCAPE_TOUCH)
                                .action(if (open) "Hide the tasks" else "Show the tasks", onFold, selected = open)
                        },
                    ),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                LabelText(
                    if (running == 1) "1 running" else "$running running",
                    tokens.type.metaSmall,
                    tokens.color.mute,
                    Modifier.weight(1f),
                )
                if (foldable) {
                    IconGlyph(
                        if (open) ConversationIcons.chevronUp else ConversationIcons.chevronDown,
                        12.dp,
                        tokens.color.mute,
                    )
                }
            }
            shown.forEach { task ->
                TaskRow(task, pane.tasks, now, onOpenTask)
            }
        }
    }
}

@Composable
private fun TaskRow(
    task: Task,
    all: List<Task>,
    now: Double,
    onOpen: (Task) -> Unit,
) {
    val tokens = Kampr.tokens
    val ended = task.state == "ended"
    val hasShell = all.any { it.parent == task.id }
    Row(
        Modifier
            .fillMaxWidth()
            .then(
                if (task.parent == null) {
                    Modifier
                } else {
                    Modifier.padding(start = 20.dp)
                },
            )
            .touchable(LANDSCAPE_TOUCH)
            .action(
                "Open ${taskLabel(task)}",
                { onOpen(task) },
                state = if (ended) "ended" else "running",
            )
            .padding(vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Mark(
            if (ended) tokens.color.mute else tokens.color.working,
            if (ended) MarkShape.Square else MarkShape.Circle,
            7.dp,
        )
        IconGlyph(
            when (task.kind) {
                "agent" -> ConversationIcons.branch
                "shell" -> KamprIcons.tool
                else -> ConversationIcons.play
            },
            12.dp,
            tokens.color.dim,
        )
        KText(
            buildList {
                add(taskLabel(task))
                if (task.backgrounded) add("backgrounded")
                if (hasShell) add("has a shell")
            }.joinToString(" · "),
            tokens.type.meta,
            tokens.color.text,
            Modifier.weight(1f),
            maxLines = 1,
        )
        if (!ended) {
            parseIsoMillis(task.since)?.let {
                elapsedSpan(now - it)?.let { span ->
                    KText(span, tokens.type.micro, tokens.color.working)
                }
            }
        }
    }
}

// The output of a watched task, on its own sheet: the lines the follow has brought, newest at
// the foot, and the end once it arrives. The follow is asked for when the sheet opens, which is
// also what a re-open re-asks for — the node owns the from-line and the sheet owns the reading.
@Composable
fun TaskOutputSheet(
    pane: PaneState,
    task: Task,
    io: PaneIo,
    breakpoint: Breakpoint,
    onDismiss: () -> Unit,
) {
    val tokens = Kampr.tokens
    val listState = rememberLazyListState()
    BottomSheet(breakpoint, onDismiss) {
        SheetHeader(taskLabel(task), "${task.kind} · ${pane.id}", null, onDismiss)
        LaunchedEffect(task.id) {
            io.send(ClientMsg.ConvoFollow(pane.id, task.id, 0))
        }
        val out = pane.taskOutput[task.id]
        val lines = out?.lines.orEmpty()
        val ended = out?.ended
        // The sheet follows the output, and a follow that flings is the one a reduced-motion
        // reader turns off: the jump is the read, the motion is decoration.
        val stillness = LocalReduceMotion.current
        LaunchedEffect(lines.size) {
            if (lines.isNotEmpty()) {
                if (stillness) listState.scrollToItem(lines.lastIndex) else listState.animateScrollToItem(lines.lastIndex)
            }
        }
        LazyColumn(
            state = listState,
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f)
                .padding(horizontal = 20.dp, vertical = 8.dp),
        ) {
            if (lines.isEmpty()) {
                item { KText("waiting for output", tokens.type.meta, tokens.color.mute) }
            }
            items(lines, key = { at -> "line:$at" }) { line ->
                KText(line, tokens.type.meta, tokens.color.text, maxLines = 8)
            }
            ended?.let {
                item {
                    KText(
                        if (it.ok) {
                            "ended${it.summary?.let { s -> ": $s" } ?: ""}"
                        } else {
                            "failed${it.summary?.let { s -> ": $s" } ?: ""}"
                        },
                        tokens.type.meta,
                        if (it.ok) tokens.color.dim else tokens.color.blocked,
                        Modifier.padding(top = 8.dp),
                    )
                }
            }
        }
    }
}

// The session sheet: what the harness wrote down about the session, and the settings and verbs
// the source listed. Every row is a description the source gave; a section with nothing in it
// takes no room, and a control with a `confirm` shows the sentence before it fires, which is the
// guard the wire exists to carry.
@Composable
fun SessionSheet(
    pane: PaneState,
    io: PaneIo,
    breakpoint: Breakpoint,
    onDismiss: () -> Unit,
) {
    var query by remember { mutableStateOf("") }
    var confirmControl by remember { mutableStateOf<Control?>(null) }
    var confirmValue by remember { mutableStateOf<JsonElement?>(null) }
    var confirmAction by remember { mutableStateOf<Action?>(null) }
    val needle = query.trim().lowercase()
    val controls = pane.controls.filter { needle.isEmpty() || it.label.lowercase().contains(needle) || it.id.lowercase().contains(needle) }
    val actions = pane.actions.filter { needle.isEmpty() || it.label.lowercase().contains(needle) || it.id.lowercase().contains(needle) }
    val facts = pane.session
    BottomSheet(breakpoint, onDismiss) {
        SheetHeader(
            "Session",
            listOfNotNull(facts?.model, facts?.version).takeIf { it.isNotEmpty() }?.joinToString(" · "),
            null,
            onDismiss,
        )
        Column(
            Modifier
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            // The facts first, and only the ones present: who pays, what the plan says, who the
            // session is. A source that has not said any of them takes no room for it.
            if (facts != null || pane.limits != null) {
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    facts?.let { f ->
                        billingWord(f.billing)?.let {
                            KText(it, Kampr.tokens.type.meta, Kampr.tokens.color.text)
                        }
                        f.account?.let {
                            KText(it, Kampr.tokens.type.meta, Kampr.tokens.color.dim)
                        }
                    }
                    pane.limits?.let { limitsWord(it) }?.let {
                        KText(it, Kampr.tokens.type.meta, Kampr.tokens.color.dim)
                    }
                }
            }
            KField(
                "search the session",
                query,
                Modifier.fillMaxWidth(),
                label = "Search the controls and actions",
                onText = { query = it },
            )
            if (controls.isNotEmpty()) {
                SheetSection("Controls")
                controls.forEach { control ->
                    ControlRow(
                        control,
                        confirming = confirmControl == control,
                        onConfirm = {
                            val c = confirmControl
                            val v = confirmValue
                            confirmControl = null
                            confirmValue = null
                            if (c != null && v != null) io.send(ClientMsg.ConvoSet(pane.id, c.id, v))
                        },
                        onPick = { value ->
                            if (control.confirm != null) {
                                confirmControl = control
                                confirmValue = value
                            } else {
                                io.send(ClientMsg.ConvoSet(pane.id, control.id, value))
                            }
                        },
                        onCancel = {
                            confirmControl = null
                            confirmValue = null
                        },
                    )
                }
            }
            if (actions.isNotEmpty()) {
                SheetSection("Actions")
                actions.forEach { action ->
                    ActionRow(
                        action,
                        confirming = confirmAction == action,
                        onFire = { args ->
                            confirmAction = null
                            io.send(ClientMsg.ConvoAct(pane.id, action.id, args))
                        },
                        onAskConfirm = { args ->
                            confirmAction = action
                            confirmValue = args
                        },
                        onCancel = {
                            confirmAction = null
                            confirmValue = null
                        },
                    )
                }
            }
            pane.acted?.let { acted ->
                KText(
                    acted.text ?: if (acted.ok) "done" else "refused",
                    Kampr.tokens.type.micro,
                    if (acted.ok) Kampr.tokens.color.dim else Kampr.tokens.color.blocked,
                )
            }
        }
    }
}

// One control, drawn by its kind. The value is what the source said it is; picking a new one
// sends `convo.set` with the source's own id, and a control with a `confirm` reads the sentence
// first — money-costing controls are the ones that carry one, and skipping the sentence is the
// defect the field exists to prevent.
@Composable
private fun ControlRow(
    control: Control,
    confirming: Boolean,
    onConfirm: () -> Unit,
    onPick: (JsonElement) -> Unit,
    onCancel: () -> Unit,
) {
    val tokens = Kampr.tokens
    var editing by remember(control.id) { mutableStateOf(false) }
    var text by remember(control.id) { mutableStateOf(control.value?.let(::valueWord) ?: "") }
    val shape = RoundedCornerShape(tokens.radii.sm)
    Column(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.surface, shape)
            .edge(tokens.card, shape)
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (confirming) {
            ConfirmRow(control.confirm.orEmpty(), onConfirm, onCancel, tokens)
            return@Column
        }
        Row(
            Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                LabelText(control.label, tokens.type.metaSmall, tokens.color.mute)
                KText(valueWord(control.value) ?: "—", tokens.type.meta, tokens.color.text, maxLines = 1)
            }
            when (control.kind) {
                "toggle" -> {
                    val v = control.value
                    val on = v is JsonPrimitive && v.booleanOrNull == true
                    KText(
                        if (on) "on" else "off",
                        tokens.type.buttonSmall,
                        if (on) tokens.color.onAccent else tokens.color.dim,
                        Modifier
                            .background(if (on) tokens.color.accent else tokens.color.raise, shape)
                            .touchable(LANDSCAPE_TOUCH)
                            .action(
                                "Turn ${control.label} ${if (on) "off" else "on"}",
                                { onPick(JsonPrimitive(!on)) },
                                shape,
                                selected = on,
                            )
                            .padding(horizontal = 12.dp, vertical = 6.dp),
                    )
                }

                else -> {
                    KText(
                        "Change",
                        tokens.type.buttonSmall,
                        tokens.color.text,
                        Modifier
                            .background(tokens.color.raise, shape)
                            .edge(tokens.card, shape)
                            .touchable(LANDSCAPE_TOUCH)
                            .action("Change ${control.label}", { editing = true }, shape)
                            .padding(horizontal = 12.dp, vertical = 6.dp),
                    )
                }
            }
        }
        when {
            confirming -> Unit

            control.kind == "choice" -> ChoiceOptions(control, onPick)

            editing -> KField(
                "a value",
                text,
                Modifier.fillMaxWidth(),
                label = "New value for ${control.label}",
                onSubmit = {
                    if (text.isNotBlank()) {
                        onPick(valueElement(control, text))
                        editing = false
                    }
                },
                onText = { text = it },
            )

            else -> Unit
        }
    }
}

// A `number` control takes a number on the wire and a `text` control takes a word, and sending
// the one for the other is a refusal the source would answer in its own words — so the value is
// built to the kind it was described as.
private fun valueElement(control: Control, text: String): JsonElement = if (control.kind == "number") {
    text.toLongOrNull()?.let { JsonPrimitive(it) } ?: JsonPrimitive(text)
} else {
    JsonPrimitive(text)
}

@Composable
private fun ChoiceOptions(control: Control, onPick: (JsonElement) -> Unit) {
    val tokens = Kampr.tokens
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        control.options.forEach { option ->
            val current = control.value?.let(::valueWord) == option.value
            Row(
                Modifier
                    .fillMaxWidth()
                    .background(if (current) tokens.color.accentSoft else tokens.color.surface2, RoundedCornerShape(tokens.radii.sm))
                    .touchable(LANDSCAPE_TOUCH)
                    .action(
                        "Set ${control.label} to ${option.label}",
                        { onPick(JsonPrimitive(option.value)) },
                        RoundedCornerShape(tokens.radii.sm),
                        selected = current,
                    )
                    .padding(horizontal = 10.dp, vertical = 7.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                KText(option.label, tokens.type.meta, tokens.color.text, Modifier.weight(1f))
                option.detail?.let {
                    KText(it, tokens.type.micro, tokens.color.mute, maxLines = 2)
                }
            }
        }
    }
}

@Composable
private fun ConfirmRow(sentence: String, onConfirm: () -> Unit, onCancel: () -> Unit, tokens: KamprTokens) {
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        KText(sentence, tokens.type.meta, tokens.color.blocked)
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PrimaryAction("Confirm", onConfirm, Modifier.weight(1f), tokens.type.buttonSmall, 8.dp, danger = true)
            PrimaryAction("Cancel", onCancel, Modifier.weight(1f), tokens.type.buttonSmall, 8.dp)
        }
    }
}

// One action, and its arguments where the source described some. An argument with options is a
// choice from the list; one without is a line. A confirm is read before the fire, the same way a
// control's is.
@Composable
private fun ActionRow(
    action: Action,
    confirming: Boolean,
    onFire: (JsonObject) -> Unit,
    onAskConfirm: (JsonObject) -> Unit,
    onCancel: () -> Unit,
) {
    val tokens = Kampr.tokens
    var editing by remember(action.id) { mutableStateOf(false) }
    var args by remember(action.id) { mutableStateOf<Map<String, String>>(emptyMap()) }
    val shape = RoundedCornerShape(tokens.radii.sm)
    val fireArgs: () -> JsonObject = {
        buildJsonObject {
            args.forEach { (k, v) -> put(k, v) }
        }
    }
    Column(
        Modifier
            .fillMaxWidth()
            .background(tokens.color.surface, shape)
            .edge(tokens.card, shape)
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (confirming) {
            ConfirmRow(
                action.confirm.orEmpty(),
                { onFire(fireArgs()) },
                onCancel,
                tokens,
            )
            return@Column
        }
        Row(
            Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                LabelText(action.label, tokens.type.metaSmall, tokens.color.mute)
                action.detail?.let {
                    KText(it, tokens.type.micro, tokens.color.dim, maxLines = 2)
                }
            }
            KText(
                if (action.args.isEmpty()) "Run" else "…",
                tokens.type.buttonSmall,
                tokens.color.onAccent,
                Modifier
                    .background(tokens.color.accent, shape)
                    .touchable(LANDSCAPE_TOUCH)
                    .action(
                        "Run ${action.label}",
                        {
                            if (action.args.isEmpty() && action.confirm == null) {
                                onFire(fireArgs())
                            } else if (action.confirm != null) {
                                onAskConfirm(fireArgs())
                            } else {
                                editing = true
                            }
                        },
                        shape,
                    )
                    .padding(horizontal = 12.dp, vertical = 6.dp),
            )
        }
        if (action.args.isNotEmpty() && editing) {
            action.args.forEach { spec ->
                ArgField(spec, args[spec.name].orEmpty()) { value ->
                    args = args + (spec.name to value)
                }
            }
            val missing = action.args.any { it.required && args[it.name].isNullOrBlank() }
            PrimaryAction(
                "Run ${action.label}",
                {
                    if (action.confirm != null) onAskConfirm(fireArgs()) else onFire(fireArgs())
                },
                Modifier.fillMaxWidth(),
                tokens.type.buttonSmall,
                8.dp,
                enabled = !missing,
            )
        }
    }
}

@Composable
private fun ArgField(spec: ArgSpec, text: String, onChange: (String) -> Unit) {
    val tokens = Kampr.tokens
    val shape = RoundedCornerShape(tokens.radii.sm)
    if (spec.options.isNotEmpty()) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            LabelText(spec.label, tokens.type.metaSmall, tokens.color.mute, Modifier.width(90.dp))
            spec.options.forEach { option ->
                val picked = text == option
                KText(
                    option,
                    tokens.type.buttonSmall,
                    if (picked) tokens.color.onAccent else tokens.color.text,
                    Modifier
                        .background(if (picked) tokens.color.accent else tokens.color.raise, shape)
                        .touchable(LANDSCAPE_TOUCH)
                        .action("Set ${spec.label} to $option", { onChange(option) }, shape, selected = picked)
                        .padding(horizontal = 10.dp, vertical = 5.dp),
                )
            }
        }
    } else {
        KField(
            spec.label,
            text,
            Modifier.fillMaxWidth(),
            label = spec.label,
            onText = onChange,
        )
    }
}

// The composer's completion chips, for a `/` the source answers with commands and a `@` it
// answers with files. Drawn only while the suggestions match the line being typed, and a tap
// replaces the word being typed with what the source said.
@Composable
fun SuggestStrip(
    items: List<Suggestion>,
    onPick: (Suggestion) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (items.isEmpty()) return
    val tokens = Kampr.tokens
    Row(
        modifier
            .fillMaxWidth()
            .horizontalScroll(rememberScrollState())
            .padding(horizontal = 12.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        items.forEach { s ->
            val shape = RoundedCornerShape(tokens.radii.pill)
            Row(
                Modifier
                    .background(tokens.color.raise, shape)
                    .edge(tokens.card, shape)
                    .touchable(LANDSCAPE_TOUCH)
                    .action("Complete with ${s.label}", { onPick(s) }, shape)
                    .padding(horizontal = 10.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(5.dp),
            ) {
                KText(s.label, tokens.type.metaSmall, tokens.color.text)
                s.detail?.let {
                    KText(it, tokens.type.micro, tokens.color.mute, maxLines = 1)
                }
            }
        }
    }
}

// The composer's row for a running turn: stop it. Send-now lives beside the box itself, where
// the line it sends is, and the stop is the nudge that needs no words. Each is drawn only where
// the source declared the word it sends — a source that cannot be interrupted has no stop to
// offer, and a row with nothing on it takes no room.
@Composable
fun ConvoSendBar(
    pane: PaneState,
    io: PaneIo,
    modifier: Modifier = Modifier,
) {
    if (pane.runState != "running") return
    val abilities = pane.abilities
    if (!abilities.canInterrupt) return
    val tokens = Kampr.tokens
    Row(
        modifier
            .fillMaxWidth()
            .background(tokens.color.bar)
            .edgeBottom()
            .padding(horizontal = 12.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        LabelText("turn running", tokens.type.metaSmall, tokens.color.working, Modifier.weight(1f))
        SendBarButton("Stop", { io.send(ClientMsg.ConvoSteer(pane.id, "interrupt")) }, tokens.color.raise, tokens.color.text, "Stop the running turn", tokens)
    }
}

@Composable
private fun SendBarButton(
    label: String,
    onClick: () -> Unit,
    bg: Color,
    fg: Color,
    say: String,
    tokens: KamprTokens,
) {
    val shape = RoundedCornerShape(tokens.radii.pill)
    KText(
        label,
        tokens.type.buttonSmall,
        fg,
        Modifier
            .background(bg, shape)
            .touchable(LANDSCAPE_TOUCH)
            .action(say, onClick, shape)
            .padding(horizontal = 12.dp, vertical = 5.dp),
    )
}
