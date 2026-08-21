package dev.kampr.gradle

import org.gradle.api.Plugin
import org.gradle.api.file.RegularFile
import org.gradle.api.initialization.Settings
import org.gradle.api.provider.ProviderFactory

// The only thing that reads `env` is the kobup publish helper, which lives outside this repository
// and is pulled in by `apply(from = …/publish-to-kobup.gradle)` from androidApp/build.gradle.kts.
// It captures `env.fetchOrNull("KOBUP_TOKEN")` at configuration time, so the extension has to be on
// every project before its build script runs. This replaces co.uzzu.dotenv, whose 4.0.0 — the last
// release — calls Project.getProperties and therefore cannot run under Gradle 10.
class DotEnvPlugin : Plugin<Settings> {
    override fun apply(settings: Settings) {
        val file = settings.layout.rootDirectory.file(".env")
        settings.gradle.lifecycle.beforeProject {
            extensions.add(DotEnv::class.java, "env", DotEnv(providers, file))
        }
    }
}

class DotEnv(private val providers: ProviderFactory, private val file: RegularFile) {
    fun fetchOrNull(name: String): String? =
        entries()[name] ?: providers.environmentVariable(name).orNull

    private fun entries(): Map<String, String> =
        providers.fileContents(file).asText.orNull?.let(::parse).orEmpty()

    private fun parse(text: String): Map<String, String> = buildMap {
        for (raw in text.lineSequence()) {
            val line = raw.trim().removePrefix("export ").trim()
            if (line.isEmpty() || line.startsWith("#")) continue
            val separator = line.indexOf('=')
            if (separator <= 0) continue
            put(line.take(separator).trim(), unquote(line.substring(separator + 1).trim()))
        }
    }

    private fun unquote(value: String): String {
        val quote = value.firstOrNull() ?: return value
        val quoted = value.length >= 2 && (quote == '"' || quote == '\'') && value.last() == quote
        return if (quoted) value.substring(1, value.length - 1) else value
    }
}
