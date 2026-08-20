package dev.kampr.terminal.guard

// This is a mistap guard, not a security control. Full command passthrough is the product and the
// threat model says so plainly: anyone who can reach a pane can already run anything on the host.
// Nothing here is a boundary — it exists so a thumb on a phone does not fire `rm -rf /` by
// accident, and every rule is bypassable in one tap by design.
data class Destructive(val reason: String, val command: String)

private class Word(val text: String, val quoted: Boolean)

// A command word is only dangerous in command position. `echo rm -rf /` and `man sudo` talk about
// a command; they do not run one, and matching them is what trains a user to tap through.
private val WRAPPERS = setOf(
    "sudo", "doas", "env", "time", "nice", "nohup", "command", "builtin", "exec", "stdbuf",
    "setsid", "xargs",
)

// A compound statement puts the real command one word in: `…; then rm -rf build` has to reach the
// rm, or a guard that only ever reads the first word misses every loop body.
private val KEYWORDS = setOf("do", "then", "else", "elif", "if", "while", "until", "!", "{", "(")

private val ELEVATORS = setOf("sudo", "doas")

private val SQL_CLIENTS = setOf(
    "psql", "mysql", "mariadb", "sqlite3", "sqlcmd", "mongosh", "duckdb", "usql", "cockroach",
    "clickhouse-client", "pgcli", "mycli",
)

private val SYSTEM_ROOTS = setOf(
    "bin", "boot", "dev", "etc", "lib", "lib32", "lib64", "opt", "proc", "root", "sbin", "srv",
    "sys", "usr", "var", "System", "Library", "Applications",
)

private val WRITABLE_DEVICES = listOf("sd", "nvme", "hd", "vd", "mmcblk", "disk", "sr", "loop", "md")

private val HARMLESS_DEVICES = setOf(
    "/dev/null", "/dev/zero", "/dev/full", "/dev/random", "/dev/urandom",
    "/dev/stdout", "/dev/stderr", "/dev/stdin", "/dev/tty",
)

private val SQL_DROP = Regex("""\bdrop\s+(table|database|schema)\b""", RegexOption.IGNORE_CASE)
private val SQL_TRUNCATE = Regex("""\btruncate\s+(table\b|\w+\s*;)""", RegexOption.IGNORE_CASE)
private val ASSIGNMENT = Regex("""^[A-Za-z_][A-Za-z0-9_]*=""")

// Separators respect quotes, and `>&` is a redirect rather than a background job, so `2>&1` stays
// with the command it belongs to instead of splitting a segment in half.
private fun segments(line: String): List<String> {
    val out = mutableListOf<String>()
    val builder = StringBuilder()
    var quote = ' '
    var index = 0
    while (index < line.length) {
        val ch = line[index]
        when {
            quote != ' ' -> {
                if (ch == quote) quote = ' '
                builder.append(ch)
                index++
            }
            ch == '\'' || ch == '"' -> {
                quote = ch
                builder.append(ch)
                index++
            }
            ch == '\\' && index + 1 < line.length -> {
                builder.append(ch).append(line[index + 1])
                index += 2
            }
            ch == '&' && index > 0 && line[index - 1] == '>' -> {
                builder.append(ch)
                index++
            }
            ch == ';' || ch == '\n' || ch == '&' || ch == '|' -> {
                out += builder.toString()
                builder.clear()
                index += if (index + 1 < line.length && line[index + 1] == ch) 2 else 1
            }
            else -> {
                builder.append(ch)
                index++
            }
        }
    }
    out += builder.toString()
    return out.map { it.trim() }.filter { it.isNotEmpty() }
}

private fun words(segment: String): List<Word> {
    val out = mutableListOf<Word>()
    val builder = StringBuilder()
    var quote = ' '
    var quoted = false
    var open = false
    var escaped = false
    for (ch in segment) {
        when {
            escaped -> {
                builder.append(ch)
                escaped = false
                open = true
            }
            ch == '\\' && quote != '\'' -> {
                escaped = true
                open = true
            }
            quote != ' ' -> if (ch == quote) quote = ' ' else builder.append(ch)
            ch == '\'' || ch == '"' -> {
                quote = ch
                quoted = true
                open = true
            }
            ch == ' ' || ch == '\t' -> {
                if (open) {
                    out += Word(builder.toString(), quoted)
                    builder.clear()
                    quoted = false
                    open = false
                }
            }
            else -> {
                builder.append(ch)
                open = true
            }
        }
    }
    if (open) out += Word(builder.toString(), quoted)
    return out
}

private fun commandAt(words: List<Word>): Int {
    var index = 0
    while (index < words.size) {
        val word = words[index]
        if (word.quoted) return index
        if (ASSIGNMENT.containsMatchIn(word.text)) {
            index++
            continue
        }
        if (word.text in KEYWORDS) {
            index++
            continue
        }
        if (word.text.substringAfterLast('/') in WRAPPERS) {
            index++
            while (index < words.size && words[index].text.startsWith("-")) index++
            continue
        }
        return index
    }
    return -1
}

private fun List<Word>.flag(short: Char, vararg long: String): Boolean = any { word ->
    if (word.quoted || !word.text.startsWith("-")) return@any false
    val body = word.text.substring(1)
    if (body.startsWith("-")) body.substring(1).substringBefore('=') in long
    else body.any { it == short }
}

private fun List<Word>.has(vararg literal: String): Boolean =
    any { !it.quoted && it.text in literal }

private fun List<Word>.operands(): List<String> =
    drop(1).filter { !it.text.startsWith("-") }.map { it.text }

// Everything under one of these is either the OS or the whole of the operator's work. A path that
// resolves to depth zero or one under them is worth naming in the confirm; anything deeper is
// still a recursive delete and still confirms, just with the quieter wording.
private fun nearRoot(path: String): Boolean {
    val bare = path.removeSuffix("*").removeSuffix("/")
    if (bare.isEmpty() || bare == "~" || bare == "\$HOME" || bare == "." || bare == "..") return true
    val home = bare.startsWith("~/") || bare.startsWith("\$HOME/")
    if (!home && !bare.startsWith("/")) return false
    val parts = bare.substringAfter('/').split('/').filter { it.isNotEmpty() }
    if (parts.isEmpty()) return true
    if (parts.size == 1) return true
    return parts.size == 2 && !home && parts.first() in SYSTEM_ROOTS
}

private fun redirectReason(words: List<Word>): String? {
    for ((index, word) in words.withIndex()) {
        if (word.quoted) continue
        val text = word.text.removePrefix(":")
        val arrow = text.indexOf('>')
        if (arrow < 0) continue
        if (!text.take(arrow).all { it.isDigit() }) continue
        if (text.getOrNull(arrow + 1) == '>' || text.getOrNull(arrow + 1) == '&') continue
        val target = text.substring(arrow + 1).ifEmpty { words.getOrNull(index + 1)?.text ?: "" }
        if (target.isEmpty() || target in HARMLESS_DEVICES) continue
        if (target.startsWith("/dev/")) {
            val node = target.removePrefix("/dev/")
            if (WRITABLE_DEVICES.any { node.startsWith(it) }) return "writes straight onto the device $target"
        }
        val root = target.removePrefix("/").substringBefore('/')
        if (target.startsWith("/") && root in SYSTEM_ROOTS) return "truncates $target"
    }
    return null
}

private fun gitSubcommand(words: List<Word>): Pair<String?, List<Word>> {
    var index = 1
    while (index < words.size) {
        val text = words[index].text
        if (text == "-C" || text == "-c") {
            index += 2
            continue
        }
        if (text.startsWith("-")) {
            index++
            continue
        }
        return text to words.drop(index + 1)
    }
    return null to emptyList()
}

private fun inspect(segment: String): Destructive? {
    if (segment.startsWith("#")) return null
    val words = words(segment)
    if (words.isEmpty()) return null
    val at = commandAt(words)
    val lead = words.take(if (at < 0) words.size else at)
    val elevated = words.size > 1 && lead.any { !it.quoted && it.text.substringAfterLast('/') in ELEVATORS }
    val argv = if (at < 0) emptyList() else words.drop(at)
    val name = argv.firstOrNull()?.text?.substringAfterLast('/').orEmpty()
    val rest = argv.drop(1)
    // A command with nothing to act on prints usage and exits. `xargs rm -rf` is the exception:
    // its operands arrive on stdin, so there is nothing on the line to see.
    val targets = argv.operands()
    val acts = targets.isNotEmpty() || lead.any { it.text == "xargs" }
    if (rest.any { !it.quoted && (it.text == "--help" || it.text == "-h") }) return null
    val reason = when {
        // -i already asks per file, which is the confirm this one exists to add.
        name == "rm" && acts && !rest.flag('i', "interactive") &&
            (rest.flag('r', "recursive") || rest.flag('R')) -> {
            val target = targets.firstOrNull { nearRoot(it) }
            if (target != null) "deletes everything under $target, recursively"
            else "deletes ${targets.firstOrNull() ?: "whatever is piped in"} and everything under it"
        }
        name.startsWith("mkfs") -> "formats a filesystem — every byte on it goes"
        name == "dd" && rest.any { !it.quoted && it.text.startsWith("of=") } ->
            "writes raw blocks straight over ${rest.first { it.text.startsWith("of=") }.text.removePrefix("of=")}"
        name == "shred" && acts -> "overwrites the file so it cannot be recovered"
        name == "truncate" && acts -> "empties the file in place"
        name == "chmod" && rest.flag('R', "recursive") &&
            targets.any { it == "777" || it == "666" || it.endsWith("+rwx") || it.endsWith("o+w") } ->
            "makes a whole tree world-writable"
        name == "git" -> gitReason(words)
        name == "docker" || name == "podman" ->
            if (argv.has("prune")) "prunes docker state — stopped containers, unused volumes and images" else null
        name == "kubectl" && argv.has("delete") && rest.none { it.text.startsWith("--dry-run") } ->
            "deletes a live cluster resource"
        else -> null
    }
    if (reason != null) return Destructive(reason, segment)
    // SQL counts only when a database client is running it, or when the line *is* SQL — the user is
    // sitting at a `psql=#` prompt. Matching the words anywhere fires on
    // `sed -i 's/DROP TABLE/keep/'` and on `grep DROP schema.sql`, and neither drops a table.
    val head = words.first()
    val bareSql = !head.quoted && head.text.uppercase() in setOf("DROP", "TRUNCATE")
    if (name in SQL_CLIENTS || bareSql) {
        if (SQL_DROP.containsMatchIn(segment)) return Destructive("drops a table or a whole database", segment)
        if (SQL_TRUNCATE.containsMatchIn(segment)) return Destructive("empties a table", segment)
    }
    redirectReason(words)?.let { return Destructive(it, segment) }
    if (elevated) return Destructive("runs as root", segment)
    return null
}

private fun gitReason(words: List<Word>): String? {
    val (subcommand, args) = gitSubcommand(words)
    return when (subcommand) {
        // --force-with-lease is the careful form and matching it is what teaches the tap-through,
        // so the comparison is on whole words rather than a prefix.
        "push" -> if (args.has("--force") || args.flag('f')) "force-pushes over shared history" else null
        "reset" -> if (args.has("--hard")) "throws away every uncommitted change" else null
        "clean" -> if (args.flag('f', "force")) "deletes untracked files for good" else null
        else -> null
    }
}

// A prompt is not one grammar — bash, zsh, starship and powerlevel all disagree — so this does not
// try to find the one true split. It offers the whole line and every suffix that follows a prompt
// marker, and the matcher runs over all of them. Carrying the prompt into a candidate costs at
// worst a false positive on that candidate; losing the command costs the feature.
private const val MARKERS = "$#%>❯➜»▶λ✗✓±→"

fun commandCandidates(line: String): List<String> {
    val out = mutableListOf<String>()
    line.trim().takeIf { it.isNotEmpty() }?.let { out += it }
    var quote = ' '
    for (index in 0 until line.length - 1) {
        val ch = line[index]
        if (quote != ' ') {
            if (ch == quote) quote = ' '
            continue
        }
        if (ch == '\'' || ch == '"') {
            quote = ch
            continue
        }
        if (ch !in MARKERS || line[index + 1] != ' ') continue
        // A leading `# ` is a comment far more often than it is a bare root prompt; a real root
        // prompt has a host or a path in front of it.
        if (ch == '#' && index == 0) continue
        val tail = line.substring(index + 1).trim()
        if (tail.isNotEmpty() && tail !in out) out += tail
    }
    return out
}

fun destructiveLine(text: String): Destructive? {
    for (line in text.split('\n')) {
        for (candidate in commandCandidates(line)) {
            for (segment in segments(candidate)) {
                inspect(segment)?.let { return Destructive(it.reason, candidate) }
            }
        }
    }
    return null
}
