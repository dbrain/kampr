import com.android.build.api.variant.AndroidComponentsExtension

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.compose.multiplatform)
}

android {
    namespace = "dev.kampr.terminal.spike.app"
    compileSdk = libs.versions.android.compileSdk.get().toInt()

    defaultConfig {
        applicationId = "dev.kampr.terminal.spike"
        minSdk = libs.versions.android.minSdk.get().toInt()
        targetSdk = libs.versions.android.targetSdk.get().toInt()
        versionCode = 1
        versionName = "0.1"
    }

    buildTypes {
        getByName("release") { isMinifyEnabled = false }
    }

    kotlin {
        jvmToolchain(21)
    }


}

// CMP 1.11.1's resource plugin does not emit Android assets for a
// com.android.kotlin.multiplatform.library target, so the APK ships them itself.
val composeResourceAssets = tasks.register<Copy>("copyComposeResourceAssets") {
    from(file("../src/commonMain/composeResources"))
    into(layout.buildDirectory.dir("generated/composeResourceAssets/composeResources/dev.kampr.terminal.spike.res"))
}

tasks.matching { it.name.startsWith("merge") && it.name.endsWith("Assets") }.configureEach {
    dependsOn(composeResourceAssets)
}

extensions.getByType(AndroidComponentsExtension::class.java).onVariants { variant ->
    val dir = layout.buildDirectory.dir("generated/composeResourceAssets").get().asFile
    dir.mkdirs()
    variant.sources.assets?.addStaticSourceDirectory(dir.relativeTo(projectDir).path)
}

dependencies {
    implementation(project(":terminal-spike"))
    implementation(libs.androidx.activity.compose)
    implementation(libs.compose.runtime)
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
}
