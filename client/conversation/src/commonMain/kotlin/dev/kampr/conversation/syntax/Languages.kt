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
    val aliases: Set<String> = emptySet(),
)

private fun words(vararg groups: String): Set<String> = groups.flatMap { it.trim().split(Regex("\\s+")) }.toSet()

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

private val RUBY = words(
    "alias and begin break case class def defined do else elsif end ensure for if in module next not",
    "or redo rescue retry return self super then undef unless until when while yield require attr_reader",
    "attr_writer attr_accessor private protected public lambda proc",
)

private val PHP = CLIKE + words("echo elseif endif endforeach endwhile foreach global include namespace new require require_once isset unset array")

private val CSHARP = CLIKE + words(
    "abstract base bool byte char checked decimal delegate double event explicit fixed float foreach goto implicit",
    "int is lock long namespace new null object params readonly ref sbyte sealed short sizeof stackalloc string",
    "uint ulong unchecked ushort using virtual volatile get set value var partial record init nameof",
)

private val SCALA = CLIKE + words("def lazy implicit given using then new forSome abstract final case extension derives")

private val LUA = words("and break do else elseif end for function goto if in local not or repeat return then until while")

private val DOCKERFILE = words("FROM AS RUN CMD LABEL EXPOSE ENV ADD COPY ENTRYPOINT VOLUME USER WORKDIR ARG ONBUILD STOPSIGNAL HEALTHCHECK SHELL")

private val MAKE = SH + words("ifeq ifneq ifdef ifndef endif define endef include override export unexport vpath")

private val PROTO = words(
    "syntax edition package import option message enum service rpc returns repeated optional required oneof",
    "map reserved stream extend extensions public weak double float int32 int64 uint32 uint64 sint32 sint64",
    "fixed32 fixed64 sfixed32 sfixed64 bool string bytes",
)

private val TRUTHS = setOf("true", "false", "null", "nil", "None", "True", "False", "undefined", "Some", "Ok", "Err")

private val HASH = listOf("#")

// Each alias is a file extension, a fence tag or a whole file name, all looked up in one place.
private val SPECS = listOf(
    LangSpec("rust", CLIKE, TRUTHS, meta = '#', aliases = setOf("rs")),
    LangSpec("kotlin", CLIKE, TRUTHS, meta = '@', aliases = setOf("kt", "kts", "gradle", "groovy")),
    LangSpec("java", CLIKE, TRUTHS, meta = '@'),
    LangSpec("swift", CLIKE, TRUTHS, meta = '@'),
    LangSpec("go", CLIKE, TRUTHS, aliases = setOf("golang")),
    LangSpec("c", CLIKE, TRUTHS, meta = '#', aliases = setOf("h")),
    LangSpec("cpp", CLIKE, TRUTHS, meta = '#', aliases = setOf("c++", "cc", "cxx", "hpp", "hh", "hxx")),
    LangSpec("csharp", CSHARP, TRUTHS, meta = '#', aliases = setOf("cs", "c#")),
    LangSpec("scala", SCALA, TRUTHS, meta = '@', aliases = setOf("sc", "sbt")),
    LangSpec("ts", CLIKE, TRUTHS, aliases = setOf("typescript", "mts", "cts")),
    LangSpec("tsx", CLIKE, TRUTHS),
    LangSpec("js", CLIKE, TRUTHS, aliases = setOf("javascript", "mjs", "cjs")),
    LangSpec("jsx", CLIKE, TRUTHS),
    LangSpec("php", PHP, TRUTHS, lineComment = listOf("//", "#"), quotes = "\"'", meta = '$'),
    LangSpec("json", emptySet(), TRUTHS, lineComment = emptyList(), blockComment = null, quotes = "\"", aliases = setOf("jsonl", "ndjson", "geojson", "webmanifest")),
    LangSpec("jsonc", emptySet(), TRUTHS, quotes = "\"", aliases = setOf("json5")),
    LangSpec("python", PY, TRUTHS, lineComment = HASH, blockComment = null, meta = '@', aliases = setOf("py", "pyi", "pyw")),
    LangSpec("ruby", RUBY, TRUTHS, lineComment = HASH, blockComment = null, meta = '@', aliases = setOf("rb", "rake", "gemspec", "gemfile", "rakefile")),
    LangSpec("lua", LUA, TRUTHS, lineComment = listOf("--"), blockComment = null, quotes = "\"'"),
    LangSpec(
        "bash",
        SH,
        TRUTHS,
        lineComment = HASH,
        blockComment = null,
        quotes = "\"'",
        meta = '$',
        aliases = setOf(
            "sh", "shell", "zsh", "ksh", "console", "env", "bashrc", "bash_profile", "bash_aliases", "profile", "zshrc",
            "zshenv", "zprofile", "pkgbuild",
        ),
    ),
    LangSpec("makefile", MAKE, TRUTHS, lineComment = HASH, blockComment = null, quotes = "\"'", meta = '$', aliases = setOf("make", "mk", "mak", "gnumakefile")),
    LangSpec("dockerfile", DOCKERFILE, TRUTHS, lineComment = HASH, blockComment = null, quotes = "\"'", meta = '$', aliases = setOf("docker", "containerfile")),
    LangSpec("yaml", words("true false null yes no on off"), TRUTHS, lineComment = HASH, blockComment = null, aliases = setOf("yml")),
    LangSpec("toml", emptySet(), TRUTHS, lineComment = HASH, blockComment = null, aliases = setOf("cargo.lock", "pipfile")),
    LangSpec(
        "ini",
        emptySet(),
        TRUTHS,
        lineComment = listOf("#", ";"),
        blockComment = null,
        quotes = "\"'",
        aliases = setOf("conf", "cfg", "properties", "gitignore", "dockerignore", "gitattributes", "gitconfig", "editorconfig", "npmrc"),
    ),
    LangSpec("sql", SQL, TRUTHS, lineComment = listOf("--"), quotes = "\"'"),
    LangSpec("proto", PROTO, TRUTHS, quotes = "\"'", aliases = setOf("protobuf")),
    LangSpec("html", emptySet(), TRUTHS, lineComment = emptyList(), blockComment = "<!--" to "-->", quotes = "\"'", aliases = setOf("htm", "xhtml")),
    LangSpec("xml", emptySet(), TRUTHS, lineComment = emptyList(), blockComment = "<!--" to "-->", quotes = "\"'", aliases = setOf("svg", "xsd", "xsl", "plist", "csproj")),
    LangSpec("css", emptySet(), TRUTHS, lineComment = emptyList(), quotes = "\"'", aliases = setOf("scss", "less")),
)

private val NAMED: Map<String, LangSpec> = buildMap {
    for (spec in SPECS) (spec.aliases + spec.name).forEach { name -> check(put(name, spec) == null) { "$name names two languages" } }
}

private val PLAIN = LangSpec("text", emptySet(), emptySet(), lineComment = emptyList(), blockComment = null, quotes = "")

// The only thing a path says about its contents: its whole name (`Makefile`, `Cargo.lock`), else
// its extension. A name with no spec is plain text rather than a guess.
fun langOf(name: String): String? {
    val base = name.substringAfterLast('/').lowercase()
    return (NAMED[base] ?: NAMED[base.substringAfterLast('.', "")])?.name
}

fun langSpec(lang: String?): LangSpec {
    val key = lang?.trim()?.lowercase()?.substringBefore(' ') ?: return PLAIN
    return NAMED[key] ?: PLAIN
}
