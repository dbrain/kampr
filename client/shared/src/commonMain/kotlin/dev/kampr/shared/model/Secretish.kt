package dev.kampr.shared.model

// Whether a command line looks like it is carrying a credential, and which word says so.
//
// **A blast-radius reduction, not a filter.** The node applies the same rule to decide what it
// writes into the fleet book by itself, and this half is what warns the operator before they save
// one on purpose — which is allowed, because a save is them saying they mean it. The two must
// agree, so both read `crates/kampr-node/tests/fixtures/secretish.json`, whose `missed` section
// names the shapes neither of them can see. `./deploy hunter2` is one of them.
//
// Kept in step with `kampr_fleet::secretish`.
fun secretish(args: List<String>): String? {
    for (argument in args) header(argument)?.let { return it }
    // Flattened, so `sh -c 'TOKEN=abc ./deploy'` — the shape this project's own docs tell the
    // operator to use for a pipeline — is read rather than treated as one opaque word.
    val words = args.flatMap { it.split(' ', '\t', '\n').filter(String::isNotEmpty) }
    words.forEachIndexed { index, token ->
        val found = assignment(token)
            ?: flagWithValue(token)
            ?: flagBeforeValue(token, words.getOrNull(index + 1))
        if (found != null) return found
    }
    return null
}

// Substring matches on an upper-cased name, so `AWS_SECRET_ACCESS_KEY` is caught by SECRET and
// `github_token` by TOKEN. `KEY` and `AUTH` are deliberately absent as bare words — they would
// catch `SORT_KEY` and `AUTHOR`, and a rule that fires on a commit author is one the operator
// learns to tap through.
private val SECRET_WORDS = listOf(
    "TOKEN", "SECRET", "PASSWORD", "PASSWD", "PASSPHRASE", "APIKEY", "API_KEY", "CREDENTIAL",
    "PRIVATE_KEY", "PRIVKEY", "BEARER", "AUTH_TOKEN",
)

// `--password-file prod.pass` names a path, and a path is the *safe* way to pass a secret. Firing
// on it would make the rule loudest exactly where the operator did the right thing.
private val PATHISH = listOf("_FILE", "_PATH")

private fun namesASecret(name: String): Boolean {
    val upper = name.replace('-', '_').uppercase()
    return PATHISH.none { upper.endsWith(it) } && SECRET_WORDS.any { upper.contains(it) }
}

// A value the shell will substitute is a reference, not a secret: `TOKEN=$CI_TOKEN` writes down
// the name of an environment variable and nothing else.
private fun isASecretValue(value: String) = value.isNotEmpty() && !value.startsWith("$")

private fun assignment(token: String): String? {
    val name = token.substringBefore('=', missingDelimiterValue = "")
    if (name.isEmpty()) return null
    if (!(name[0].isAsciiLetter() || name[0] == '_')) return null
    if (!name.all { it.isAsciiLetter() || it in '0'..'9' || it == '_' }) return null
    val value = token.substring(name.length + 1)
    return if (isASecretValue(value) && namesASecret(name)) name else null
}

private fun flagWithValue(token: String): String? {
    if (!token.startsWith("-") || !token.contains('=')) return null
    val flag = token.substringBefore('=')
    val value = token.substring(flag.length + 1)
    return if (isASecretValue(value) && namesASecret(flag.trimStart('-'))) flag else null
}

// A long flag whose value is the next word. Long only: `-p` is a port at least as often as a
// password, and a rule that fires on `ssh -p 2222` is one nobody believes.
private fun flagBeforeValue(token: String, next: String?): String? {
    if (next == null || !token.startsWith("--") || token.contains('=') || next.startsWith("-")) return null
    return if (isASecretValue(next) && namesASecret(token.trimStart('-'))) token else null
}

// `curl -H "Authorization: Bearer …"`, the one very common shape carrying no `=` at all. Read off
// the whole argument rather than its words: the header arrives inside one quoted string, and
// splitting it puts the scheme and the credential in different tokens.
private fun header(argument: String): String? {
    val lower = argument.lowercase()
    for ((needle, said) in listOf("authorization:" to "Authorization:", "bearer " to "Bearer")) {
        val at = lower.indexOf(needle)
        if (at < 0) continue
        val rest = argument.substring(at + needle.length).trim(' ', '\t', '"', '\'')
        // A header whose credential is `$TOKEN` names a variable; the secret is in the environment
        // and never in what would be written down.
        if (rest.isNotEmpty() && !rest.contains('$')) return said
    }
    return null
}

private fun Char.isAsciiLetter() = this in 'a'..'z' || this in 'A'..'Z'
