import org.jetbrains.kotlin.gradle.ExperimentalWasmDsl

plugins {
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.compose.multiplatform)
}

// The boot background is painted before wasm loads, so it cannot come from the Compose token
// layer at runtime — it is extracted from it at build time instead, so the two cannot drift.
// Both grounds are extracted: with only the dark one a light-ground load flashes black.
val tokensFile = layout.projectDirectory.file(
    "../shared/src/commonMain/kotlin/dev/kampr/shared/theme/Themes.kt"
)

val generateBootCss = tasks.register("generateBootCss") {
    val source = tokensFile
    val outputDir = layout.buildDirectory.dir("generated/bootCss")
    inputs.file(source)
    outputs.dir(outputDir)
    doLast {
        val text = source.asFile.readText()
        val soft = text.substringAfter("val SoftFamily").substringBefore("val PhosphorFamily")
        val pattern = Regex("""bg = Color\(0xFF([0-9A-Fa-f]{6})\)""")
        fun ground(name: String): String {
            val block = soft.substringAfter("$name = Palette(")
            return pattern.find(block)?.groupValues?.get(1)
                ?: error("SoftFamily $name bg token not found in ${source.asFile}; boot CSS cannot be generated")
        }
        val dark = ground("dark")
        val light = ground("light")
        outputDir.get().asFile.mkdirs()
        outputDir.get().file("kampr.css").asFile.writeText(
            """
            html, body { margin: 0; padding: 0; width: 100%; height: 100%; overflow: hidden; background: #$dark; }
            @media (prefers-color-scheme: light) { html, body { background: #$light; } }
            html[data-ground="dark"], html[data-ground="dark"] body { background: #$dark; }
            html[data-ground="light"], html[data-ground="light"] body { background: #$light; }
            canvas { outline: none; }
            """.trimIndent() + "\n"
        )
    }
}

kotlin {
    jvmToolchain(21)

    @OptIn(ExperimentalWasmDsl::class)
    wasmJs {
        outputModuleName.set("kamprWeb")
        browser {
            commonWebpackConfig {
                outputFileName = "kamprWeb.js"
            }
        }
        binaries.executable()
    }

    sourceSets {
        named("wasmJsMain") { resources.srcDir(generateBootCss) }
        wasmJsMain.dependencies {
            implementation(project(":shared"))
            implementation(project(":terminal"))
            implementation(project(":conversation"))
        implementation(project(":mosaic"))
            implementation(libs.compose.runtime)
            implementation(libs.compose.ui)
            implementation(libs.compose.foundation)
            implementation(libs.kotlinx.browser)
        }
    }
}

// The single-binary join: rust-embed bakes `crates/kampr-node/dist/` into the node, and this is
// what puts the wasm bundle there. Sync, not Copy — a stale hashed .wasm left behind would be
// served forever under its old name.
val stageNodeBundle = tasks.register<Sync>("stageNodeBundle") {
    from(tasks.named("wasmJsBrowserDistribution"))
    into(rootProject.layout.projectDirectory.dir("../crates/kampr-node/dist"))
    preserve { include(".gitkeep") }
}

tasks.named("build") { dependsOn(stageNodeBundle) }
