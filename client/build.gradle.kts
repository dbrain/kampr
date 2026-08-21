plugins {
    base
    alias(libs.plugins.kotlin.multiplatform) apply false
    alias(libs.plugins.kotlin.jvm) apply false
    alias(libs.plugins.kotlin.compose) apply false
    alias(libs.plugins.kotlin.serialization) apply false
    alias(libs.plugins.compose.multiplatform) apply false
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.android.kmp.library) apply false
}

// The `env` extension has one consumer and it is not in this tree, so its tests are the only thing
// that would notice it breaking.
tasks.named("check") {
    dependsOn(gradle.includedBuild("dotenv").task(":check"))
}
