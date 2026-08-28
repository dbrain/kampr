package dev.kampr.shared.model

import dev.kampr.shared.wire.PaneInfo

// The Kotlin half of `crates/kampr-core/src/naming.rs`, and it is a port rather than a second
// design: the same template over the same fields has to render the same string here as it does in
// the CLI and in the node, or a pane is called two things depending on which screen it is on.
// `crates/kampr-core/tests/fixtures/naming-cases.json` is what holds the two to each other, and
// `NamingParityTest` is what reads it from this side.
//
// Two shapes and no more. `{a|b|'x'}` takes the first of its choices that resolves to something,
// and `[…]` is dropped whole when nothing inside it did — which exists because `{cmd}` is blank on
// every pane of a machine that sources ble.sh (probe #297) and `kampr ()` is worse than `kampr`.
object Naming {
    const val DEFAULT_TEMPLATE = "{label|workspace|cwd|pane}[ ({argv|cmd})] · {agent|'bash'}"

    val default: Template by lazy { Template.parse(DEFAULT_TEMPLATE) }
}

class TemplateException(message: String) : IllegalArgumentException(message)

data class Fields(
    val pane: String,
    val workspace: String? = null,
    val tab: String? = null,
    // The whole path. `{cwd}` renders its last segment.
    val cwd: String? = null,
    val label: String? = null,
    val agent: String? = null,
    val status: AgentStatus = AgentStatus.Unknown,
    val cmd: String? = null,
    val argv: String? = null,
)

fun fieldsOf(pane: PaneInfo): Fields = Fields(
    pane = pane.id.substringAfter('/'),
    workspace = pane.workspace,
    tab = pane.tab,
    cwd = pane.cwd,
    label = pane.label,
    agent = pane.agent,
    status = statusOf(pane),
    cmd = pane.cmd,
    argv = pane.argv,
)

class Template private constructor(private val parts: List<Part>) {
    companion object {
        fun parse(source: String): Template {
            val scan = Scanner(source)
            val parts = scan.parts(inGroup = false)
            return Template(parts)
        }
    }

    // Never empty: a template that resolves to nothing gives the pane's own id back, because a
    // nameless row in a sidebar is not something an operator can act on.
    fun render(fields: Fields): String {
        val out = StringBuilder()
        renderInto(parts, fields, out)
        val name = out.toString().trim()
        return if (name.isEmpty()) fields.pane else name
    }
}

private sealed interface Part {
    data class Text(val text: String) : Part
    data class Slot(val choices: List<Choice>) : Part
    data class Group(val parts: List<Part>) : Part
}

private sealed interface Choice {
    data class Token(val token: Field) : Choice
    data class Literal(val text: String) : Choice
}

private enum class Field { Label, Workspace, Tab, Cwd, Pane, Agent, Status, Cmd, Argv }

private fun field(name: String): Field? = when (name) {
    "label" -> Field.Label
    "workspace" -> Field.Workspace
    "tab" -> Field.Tab
    "cwd" -> Field.Cwd
    "pane" -> Field.Pane
    "agent" -> Field.Agent
    "status" -> Field.Status
    "cmd" -> Field.Cmd
    "argv" -> Field.Argv
    else -> null
}

private class Scanner(private val source: String) {
    private var at = 0

    private fun peek(): Char? = source.getOrNull(at)

    private fun next(): Char? = source.getOrNull(at)?.also { at++ }

    fun parts(inGroup: Boolean): List<Part> {
        val parts = mutableListOf<Part>()
        val text = StringBuilder()
        fun flush() {
            if (text.isNotEmpty()) {
                parts.add(Part.Text(text.toString()))
                text.clear()
            }
        }
        while (true) {
            val c = next() ?: break
            when {
                c == '{' && peek() == '{' -> { at++; text.append('{') }
                c == '}' && peek() == '}' -> { at++; text.append('}') }
                c == '[' && peek() == '[' -> { at++; text.append('[') }
                c == ']' && peek() == ']' -> { at++; text.append(']') }
                c == '{' -> { flush(); parts.add(Part.Slot(slot())) }
                c == '[' -> { flush(); parts.add(Part.Group(parts(inGroup = true))) }
                c == ']' && inGroup -> { flush(); return parts }
                c == ']' -> throw TemplateException("a `]` closes a group that was never opened")
                else -> text.append(c)
            }
        }
        if (inGroup) throw TemplateException("a `[` was never closed")
        flush()
        return parts
    }

    private fun slot(): List<Choice> {
        val choices = mutableListOf<Choice>()
        val word = StringBuilder()
        var literal: StringBuilder? = null
        var closed = false
        while (true) {
            val c = next() ?: break
            val open = literal
            when {
                c == '\'' && open != null -> { choices.add(Choice.Literal(open.toString())); literal = null; word.clear() }
                c == '\'' -> literal = StringBuilder()
                open != null -> open.append(c)
                c == '|' || c == '}' -> {
                    val name = word.toString().trim()
                    if (name.isNotEmpty()) {
                        val token = field(name) ?: throw TemplateException(
                            "no such template token `$name`; the tokens are label, workspace, tab, cwd, " +
                                "pane, agent, status, cmd, argv",
                        )
                        choices.add(Choice.Token(token))
                    }
                    word.clear()
                    if (c == '}') { closed = true; break }
                }
                else -> word.append(c)
            }
        }
        if (literal != null) throw TemplateException("a `'` was never closed")
        if (!closed) throw TemplateException("a `{` was never closed")
        if (choices.isEmpty()) throw TemplateException("a `{}` needs at least one token or 'literal' in it")
        return choices
    }
}

private class Filled(var seen: Boolean = false, var any: Boolean = false)

private fun renderInto(parts: List<Part>, fields: Fields, out: StringBuilder): Filled {
    val filled = Filled()
    for (part in parts) {
        when (part) {
            is Part.Text -> out.append(part.text)
            is Part.Slot -> {
                filled.seen = true
                for (choice in part.choices) {
                    val resolved = when (choice) {
                        is Choice.Token -> value(choice.token, fields)
                        is Choice.Literal -> choice.text
                    }
                    if (resolved != null) {
                        out.append(resolved)
                        filled.any = true
                        break
                    }
                }
            }
            is Part.Group -> {
                val buffer = StringBuilder()
                val inner = renderInto(part.parts, fields, buffer)
                // A group with no slots in it is prose, not a conditional, so it stays.
                if (inner.any || !inner.seen) {
                    out.append(buffer)
                    filled.any = filled.any || inner.any
                    filled.seen = filled.seen || inner.seen
                }
            }
        }
    }
    return filled
}

private fun value(token: Field, fields: Fields): String? {
    val raw = when (token) {
        Field.Label -> fields.label
        Field.Workspace -> fields.workspace
        Field.Tab -> fields.tab
        Field.Cwd -> fields.cwd?.let(::basename)
        Field.Pane -> fields.pane
        Field.Agent -> fields.agent
        Field.Status -> statusWord(fields.status)
        Field.Cmd -> fields.cmd
        Field.Argv -> fields.argv
    }
    return raw?.trim()?.takeIf { it.isNotEmpty() }
}

private fun basename(path: String): String {
    val trimmed = path.trimEnd('/')
    return if (trimmed.isEmpty()) path else trimmed.substringAfterLast('/')
}

// Not a state to print. It is the absence of one, and a row that says so says nothing.
private fun statusWord(status: AgentStatus): String? = when (status) {
    AgentStatus.Idle -> "idle"
    AgentStatus.Working -> "working"
    AgentStatus.Blocked -> "blocked"
    AgentStatus.Done -> "done"
    AgentStatus.Unknown -> null
}
