package dev.kampr.conversation

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

// The node's own summary keys (kampr-journal's `summary.rs`), in its order, so a call reads the
// same while it is being written as once it is recorded.
private val SUMMARY_KEYS = listOf("description", "file_path", "path", "pattern", "query", "url", "cmd", "command", "prompt")

private const val LONGEST = 120

// A live tool's arguments arrive as the JSON the model is still writing, so most reads are of an
// object with no closing brace. A whole object is read as one; a partial one is scanned for the
// first summary key whose string has started, and the words so far are the summary.
fun liveArgsSummary(args: String): String? {
    if (args.isBlank()) return null
    val whole = runCatching { Json.parseToJsonElement(args) as? JsonObject }.getOrNull()
    val said = if (whole != null) {
        SUMMARY_KEYS.firstNotNullOfOrNull { key -> (whole[key] as? JsonPrimitive)?.takeIf { it.isString }?.content }
    } else {
        SUMMARY_KEYS.firstNotNullOfOrNull { key -> partialString(args, key) }
    }
    return said?.lineSequence()?.firstOrNull()?.trim()?.takeIf { it.isNotEmpty() }?.let {
        if (it.length <= LONGEST) it else it.take(LONGEST - 1) + "…"
    }
}

private fun partialString(args: String, key: String): String? {
    val found = Regex("\"" + key + "\"\\s*:\\s*\"").find(args) ?: return null
    val out = StringBuilder()
    var at = found.range.last + 1
    while (at < args.length) {
        val c = args[at]
        when {
            c == '"' -> return out.toString()

            c == '\\' && at + 1 < args.length -> {
                when (val next = args[at + 1]) {
                    'n' -> out.append('\n')
                    't' -> out.append('\t')
                    else -> out.append(next)
                }
                at++
            }

            c == '\\' -> break

            else -> out.append(c)
        }
        at++
    }
    return out.toString()
}
