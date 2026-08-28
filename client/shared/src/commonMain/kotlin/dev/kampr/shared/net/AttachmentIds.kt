package dev.kampr.shared.net

import kotlin.io.encoding.Base64

// The separator every attachment id has used since the first build that minted one. Not a path
// separator, not legal in a JSON string, and not something a filename can hold.
private const val SEPARATOR = '\u001F'

// A record id is five separator-delimited fields, so the **number** of fields is what tells the
// forms apart and every id an installed client is holding decodes to what it always did. These two
// are the forms a *client* builds: it saw a path in a tool call and wants what is at it.
private const val FILE = "file"
private const val DIFF = "diff"

// The node's own ceiling on an id, applied here as well so a path that could never resolve is
// never offered as though it could.
private const val LONGEST_ID = 4096

// base64url with no padding, which is the alphabet the route reads and leaves an id as one path
// segment with nothing in it a URL minds.
private fun mint(tag: String, path: String): String =
    Base64.UrlSafe.encode("$tag$SEPARATOR$path".encodeToByteArray()).trimEnd('=')

fun fileAttachmentId(path: String): String = mint(FILE, path)

// The same path, asked about rather than read: what git says has changed in it since HEAD.
fun diffAttachmentId(path: String): String = mint(DIFF, path)

// The path an id this client minted names, which is how the file viewer asks for the diff beside
// the file without being handed the path twice. A record id — five fields, minted by a node —
// answers null, because there is no path in one.
fun pathOfAttachmentId(id: String): String? {
    if (id.isEmpty() || id.length > LONGEST_ID) return null
    val padded = id + "=".repeat((4 - id.length % 4) % 4)
    val text = runCatching { Base64.UrlSafe.decode(padded).decodeToString() }.getOrNull() ?: return null
    val fields = text.split(SEPARATOR)
    if (fields.size != 2) return null
    val (tag, path) = fields
    return path.takeIf { it.isNotEmpty() && (tag == FILE || tag == DIFF) }
}

// A path the **node** derived and a reader cannot dispute — a tool card's summary, filled from
// `file_path`/`path` for every Read, Edit and Write, or a diff block's own path. Absolute or
// anchored at the node's home, because those are the only two the route resolves: a relative path
// is refused there rather than guessed at against whatever directory the node happens to have been
// started in.
//
// Deliberately not a search through prose. Detecting a path in a sentence is a guess about
// English, and a guess that offers to fetch a file is worse than not offering one.
fun filePathOf(text: String?): String? {
    val path = text?.trim() ?: return null
    if (path.length > LONGEST_ID / 2) return null
    if (path.any { it.code < 0x20 }) return null
    if (path.endsWith('/')) return null
    return path.takeIf { it.startsWith('/') || it.startsWith("~/") }
}
