import org.jetbrains.kotlin.gradle.ExperimentalWasmDsl

plugins {
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.compose.multiplatform)
}

// The boot background is painted before wasm loads, so it cannot come from the Compose token
// layer at runtime — it is extracted from it at build time instead, so the two cannot drift.
val tokensFile = layout.projectDirectory.file(
    "../shared/src/commonMain/kotlin/dev/kampr/shared/theme/Tokens.kt"
)

val generateBootCss = tasks.register("generateBootCss") {
    val source = tokensFile
    val outputDir = layout.buildDirectory.dir("generated/bootCss")
    inputs.file(source)
    outputs.dir(outputDir)
    doLast {
        val text = source.asFile.readText()
        val soft = text.substringAfter("val SoftTheme").substringBefore("val PhosphorTheme")
        val hex = Regex("""bg = Color\(0xFF([0-9A-Fa-f]{6})\)""").find(soft)?.groupValues?.get(1)
            ?: error("SoftTheme bg token not found in ${source.asFile}; boot CSS cannot be generated")
        outputDir.get().asFile.mkdirs()
        outputDir.get().file("kampr.css").asFile.writeText(
            """
            html, body { margin: 0; padding: 0; width: 100%; height: 100%; overflow: hidden; background: #$hex; }
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
            implementation(libs.compose.runtime)
            implementation(libs.compose.ui)
            implementation(libs.compose.foundation)
            implementation(libs.kotlinx.browser)
        }
    }
}
