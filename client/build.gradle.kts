plugins {
    base
    alias(libs.plugins.kotlin.multiplatform) apply false
    alias(libs.plugins.kotlin.jvm) apply false
    alias(libs.plugins.kotlin.compose) apply false
    alias(libs.plugins.kotlin.serialization) apply false
    alias(libs.plugins.compose.multiplatform) apply false
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.android.kmp.library) apply false
    alias(libs.plugins.ktlint) apply false
}

// The baseline holds every finding that predates the lint, so `check` is green on the code as it
// stands and fails on anything new. Shrink it by fixing a file and regenerating with
// `./gradlew ktlintGenerateBaseline`; never grow it to make a change pass.
val ktlintVersion = libs.versions.ktlint.cli.get()

subprojects {
    apply(plugin = "org.jlleitschuh.gradle.ktlint")
    extensions.configure<org.jlleitschuh.gradle.ktlint.KtlintExtension> {
        version.set(ktlintVersion)
        baseline.set(rootProject.file("config/ktlint/baseline-${path.removePrefix(":").replace(':', '-')}.xml"))
    }
}
