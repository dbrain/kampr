package dev.kampr.conversation.syntax

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class LanguagesTest {
    @Test
    fun aFileIsKnownByItsExtensionOrItsWholeName() {
        val table = listOf(
            "main.rs" to "rust",
            "/home/u/src/Main.kt" to "kotlin",
            "build.gradle.kts" to "kotlin",
            "build.gradle" to "kotlin",
            "vec.h" to "c",
            "vec.hpp" to "cpp",
            "vec.cc" to "cpp",
            "app.mjs" to "js",
            "app.cjs" to "js",
            "app.ts" to "ts",
            "run.bash" to "bash",
            "events.jsonl" to "json",
            "events.ndjson" to "json",
            "page.htm" to "html",
            "logo.svg" to "xml",
            "site.scss" to "css",
            "deploy.yml" to "yaml",
            "tool.py" to "python",
            "Makefile" to "makefile",
            "rules.mk" to "makefile",
            "Dockerfile" to "dockerfile",
            ".bashrc" to "bash",
            ".zshrc" to "bash",
            "prod.env" to "bash",
            ".env" to "bash",
            ".gitignore" to "ini",
            "setup.cfg" to "ini",
            "Cargo.lock" to "toml",
            "Gemfile" to "ruby",
            "app.rb" to "ruby",
            "index.php" to "php",
            "Program.cs" to "csharp",
            "Main.scala" to "scala",
            "init.lua" to "lua",
            "wire.proto" to "proto",
            "notes.txt" to null,
            "README" to null,
            "archive.lock" to null,
        )
        for ((name, lang) in table) assertEquals(lang, langOf(name), name)
    }

    @Test
    fun aFenceTagIsReadThroughTheSameNames() {
        val table = listOf(
            "rs" to "rust",
            "Rust" to "rust",
            "rust ignore" to "rust",
            "kts" to "kotlin",
            "kt" to "kotlin",
            "ts" to "ts",
            "typescript" to "ts",
            "py" to "python",
            "yml" to "yaml",
            "sh" to "bash",
            "console" to "bash",
            "c++" to "cpp",
            "c#" to "csharp",
            "docker" to "dockerfile",
            "nonsense" to "text",
        )
        for ((tag, lang) in table) assertEquals(lang, langSpec(tag).name, tag)
    }

    @Test
    fun eachAddedLanguageMarksItsOwnWords() {
        val table = listOf(
            Triple("ruby", "def go\n  @x = 1 # why\nend", listOf(Token.Keyword to "def", Token.Comment to "# why")),
            Triple("php", "function go() { return \$x; } # why", listOf(Token.Keyword to "function", Token.Meta to "\$x", Token.Comment to "# why")),
            Triple("csharp", "namespace App { public sealed class A {} }", listOf(Token.Keyword to "namespace", Token.Keyword to "class")),
            Triple("scala", "def go(x: Int) = x // why", listOf(Token.Keyword to "def", Token.Comment to "// why")),
            Triple("lua", "local function go() end -- why", listOf(Token.Keyword to "local", Token.Keyword to "function", Token.Comment to "-- why")),
            Triple("dockerfile", "FROM debian\nRUN echo \$HOME # why", listOf(Token.Keyword to "FROM", Token.Meta to "\$HOME", Token.Comment to "# why")),
            Triple("makefile", "ifeq (\$(X),1)\nendif # why", listOf(Token.Keyword to "ifeq", Token.Comment to "# why")),
            Triple("ini", "; why\n# also\nkey = \"v\"", listOf(Token.Comment to "; why", Token.Comment to "# also", Token.Text to "\"v\"")),
            Triple("proto", "message Pane { repeated string id = 1; }", listOf(Token.Keyword to "message", Token.Keyword to "repeated")),
        )
        for ((lang, code, expected) in table) {
            val found = scan(code, langSpec(lang)).map { it.token to code.substring(it.start, it.end) }
            for (want in expected) assertTrue(want in found, "$lang: $want not in $found")
        }
    }
}
