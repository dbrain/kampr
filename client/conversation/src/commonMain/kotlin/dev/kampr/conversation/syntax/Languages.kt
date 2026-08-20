package dev.kampr.conversation.syntax

enum class Token { Plain, Keyword, Text, Number, Comment, Punct, Call, Meta }

data class LangSpec(
    val name: String,
    val keywords: Set<String>,
    val constants: Set<String> = emptySet(),
    val lineComment: List<String> = listOf("//"),
    val blockComment: Pair<String, String>? = "/*" to "*/",
    val quotes: String = "\"'`",
    val meta: Char? = null,
)

private fun words(vararg groups: String): Set<String> =
    groups.flatMap { it.trim().split(Regex("\\s+")) }.toSet()

private val CLIKE = words(
    "as async await break case catch class const continue crate default defer do dyn else enum export",
    "extends extern fn for from fun func function go if impl implements import in interface let loop",
    "match mod module move mut package private protected pub public return self static struct super",
    "switch trait try type typeof union unsafe use val var void where while with yield object companion",
    "override suspend data sealed internal operator inline reified vararg lateinit init when is out",
)

private val PY = words(
    "and as assert async await break class continue def del elif else except finally for from global",
    "if import in is lambda nonlocal not or pass raise return try while with yield match case",
)

private val SH = words(
    "if then else elif fi for while until do done case esac function select in return exit export",
    "local readonly set unset shift source trap eval exec test time coproc declare",
)

private val SQL = words(
    "select from where group by having order limit offset insert into values update set delete",
    "create table index view drop alter add column primary key foreign references join left right",
    "inner outer on as distinct union all and or not null is between like exists with returning",
)

private val TRUTHS = setOf("true", "false", "null", "nil", "None", "True", "False", "undefined", "Some", "Ok", "Err")

private val SPECS = listOf(
    LangSpec("rust", CLIKE, TRUTHS, meta = '#'),
    LangSpec("kotlin", CLIKE, TRUTHS, meta = '@'),
    LangSpec("java", CLIKE, TRUTHS, meta = '@'),
    LangSpec("kt", CLIKE, TRUTHS, meta = '@'),
    LangSpec("swift", CLIKE, TRUTHS, meta = '@'),
    LangSpec("go", CLIKE, TRUTHS),
    LangSpec("c", CLIKE, TRUTHS, meta = '#'),
    LangSpec("cpp", CLIKE, TRUTHS, meta = '#'),
    LangSpec("ts", CLIKE, TRUTHS),
    LangSpec("tsx", CLIKE, TRUTHS),
    LangSpec("js", CLIKE, TRUTHS),
    LangSpec("jsx", CLIKE, TRUTHS),
    LangSpec("json", emptySet(), TRUTHS, lineComment = emptyList(), blockComment = null, quotes = "\""),
    LangSpec("jsonc", emptySet(), TRUTHS, quotes = "\""),
    LangSpec("python", PY, TRUTHS, lineComment = listOf("#"), blockComment = null, meta = '@'),
    LangSpec("py", PY, TRUTHS, lineComment = listOf("#"), blockComment = null, meta = '@'),
    LangSpec("bash", SH, TRUTHS, lineComment = listOf("#"), blockComment = null, quotes = "\"'", meta = '$'),
    LangSpec("sh", SH, TRUTHS, lineComment = listOf("#"), blockComment = null, quotes = "\"'", meta = '$'),
    LangSpec("shell", SH, TRUTHS, lineComment = listOf("#"), blockComment = null, quotes = "\"'", meta = '$'),
    LangSpec("zsh", SH, TRUTHS, lineComment = listOf("#"), blockComment = null, quotes = "\"'", meta = '$'),
    LangSpec("console", SH, TRUTHS, lineComment = listOf("#"), blockComment = null, quotes = "\"'", meta = '$'),
    LangSpec("yaml", words("true false null yes no on off"), TRUTHS, lineComment = listOf("#"), blockComment = null),
    LangSpec("yml", words("true false null yes no on off"), TRUTHS, lineComment = listOf("#"), blockComment = null),
    LangSpec("toml", emptySet(), TRUTHS, lineComment = listOf("#"), blockComment = null),
    LangSpec("sql", SQL, TRUTHS, lineComment = listOf("--"), quotes = "\"'"),
    LangSpec("html", emptySet(), TRUTHS, lineComment = emptyList(), blockComment = "<!--" to "-->", quotes = "\"'"),
    LangSpec("xml", emptySet(), TRUTHS, lineComment = emptyList(), blockComment = "<!--" to "-->", quotes = "\"'"),
    LangSpec("css", emptySet(), TRUTHS, lineComment = emptyList(), quotes = "\"'"),
)

private val PLAIN = LangSpec("text", emptySet(), emptySet(), lineComment = emptyList(), blockComment = null, quotes = "")

fun langSpec(lang: String?): LangSpec {
    val key = lang?.trim()?.lowercase()?.substringBefore(' ') ?: return PLAIN
    return SPECS.firstOrNull { it.name == key } ?: PLAIN
}
